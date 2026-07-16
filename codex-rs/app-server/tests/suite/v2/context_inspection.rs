use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use app_test_support::TestAppServer;
use app_test_support::create_final_assistant_message_sse_response;
use app_test_support::to_response;
use codex_app_server_protocol::ContextContentDisclosure;
use codex_app_server_protocol::ContextInspectionLimits;
use codex_app_server_protocol::DynamicToolFunctionSpec;
use codex_app_server_protocol::DynamicToolSpec;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::ModelRequestAttemptStatus;
use codex_app_server_protocol::ModelRequestTransport;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadContextInspectParams;
use codex_app_server_protocol::ThreadContextInspectResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::UserInput;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio::time::timeout;

#[cfg(windows)]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(25);
#[cfg(not(windows))]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(10);
const PRIVATE_USER_TEXT: &str = "context-inspection-private-user-text";
const PRIVATE_BASE_INSTRUCTIONS: &str = "context-inspection-private-base-instructions";
const PRIVATE_TOOL_SCHEMA_TEXT: &str = "context-inspection-private-tool-schema";
const PRIVATE_OUTPUT_SCHEMA_TEXT: &str = "context-inspection-private-output-schema";

#[tokio::test]
async fn thread_context_inspect_is_bounded_metadata_only_and_read_only() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_once(
        &server,
        create_final_assistant_message_sse_response("Done")?,
    )
    .await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut app_server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build()
        .await?;
    timeout(DEFAULT_READ_TIMEOUT, app_server.initialize()).await??;

    let start_request_id = app_server
        .send_thread_start_request_with_auto_env(ThreadStartParams {
            base_instructions: Some(PRIVATE_BASE_INSTRUCTIONS.to_string()),
            dynamic_tools: Some(vec![DynamicToolSpec::Function(DynamicToolFunctionSpec {
                name: "context_inspection_test_tool".to_string(),
                description: PRIVATE_TOOL_SCHEMA_TEXT.to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "secret": {
                            "type": "string",
                            "description": PRIVATE_TOOL_SCHEMA_TEXT,
                        }
                    },
                    "additionalProperties": false,
                }),
                defer_loading: false,
            })]),
            ..Default::default()
        })
        .await?;
    let start_response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_response_message(RequestId::Integer(start_request_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response(start_response)?;

    let turn_request_id = app_server
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: PRIVATE_USER_TEXT.to_string(),
                text_elements: Vec::new(),
            }],
            output_schema: Some(serde_json::json!({
                "type": "object",
                "description": PRIVATE_OUTPUT_SCHEMA_TEXT,
                "properties": {
                    "answer": { "type": "string" }
                },
                "required": ["answer"],
                "additionalProperties": false,
            })),
            ..Default::default()
        })
        .await?;
    let turn_response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_response_message(RequestId::Integer(turn_request_id)),
    )
    .await??;
    let _: TurnStartResponse = to_response(turn_response)?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    let outbound_request = serde_json::to_string(&response_mock.single_request().body_json())?;
    for private_text in [
        PRIVATE_USER_TEXT,
        PRIVATE_BASE_INSTRUCTIONS,
        PRIVATE_TOOL_SCHEMA_TEXT,
        PRIVATE_OUTPUT_SCHEMA_TEXT,
    ] {
        assert!(outbound_request.contains(private_text));
    }

    let (first, first_wire_response) =
        inspect_context(&mut app_server, &thread.id, Some(/*max_items*/ 0)).await?;
    let (second, _) = inspect_context(&mut app_server, &thread.id, Some(/*max_items*/ 0)).await?;
    assert_eq!(first, second);
    for private_text in [
        PRIVATE_USER_TEXT,
        PRIVATE_BASE_INSTRUCTIONS,
        PRIVATE_TOOL_SCHEMA_TEXT,
        PRIVATE_OUTPUT_SCHEMA_TEXT,
    ] {
        assert!(!first_wire_response.contains(private_text));
    }
    assert_eq!(
        first.snapshot.content_disclosure,
        ContextContentDisclosure::MetadataOnly
    );
    assert_eq!(
        first.snapshot.applied_limits,
        ContextInspectionLimits { max_items: 0 }
    );
    assert_eq!(first.snapshot.raw.items, Vec::new());
    assert_eq!(first.snapshot.normalized.items, Vec::new());
    assert!(first.snapshot.raw.total_items > 0);
    assert!(first.snapshot.normalized.total_items > 0);
    let latest_attempt = first
        .snapshot
        .request
        .latest_attempt
        .as_ref()
        .expect("latest request attempt");
    let last_actual = first
        .snapshot
        .request
        .last_actual
        .as_ref()
        .expect("last actual request");
    assert_eq!(latest_attempt, last_actual);
    assert_eq!(
        (
            latest_attempt.status,
            latest_attempt.transport,
            latest_attempt.transport_uses_delta,
            latest_attempt.connection_reused,
        ),
        (
            ModelRequestAttemptStatus::Sent,
            ModelRequestTransport::ResponsesHttp,
            false,
            false,
        )
    );
    assert_eq!(
        first
            .snapshot
            .request
            .current_normalized_matches_last_actual,
        Some(true)
    );
    assert_eq!(latest_attempt.normalized_input.items, Vec::new());
    assert_eq!(latest_attempt.provider_input.items, Vec::new());
    assert!(latest_attempt.normalized_input.total_items > 0);
    assert!(latest_attempt.provider_input.total_items > 0);
    assert!(
        latest_attempt
            .instructions
            .as_ref()
            .is_some_and(|instructions| instructions.serialized_bytes > 0)
    );
    assert!(
        latest_attempt
            .tools
            .as_ref()
            .is_some_and(|tools| tools.total_items > 0 && tools.serialized_bytes > 0)
    );
    assert!(
        latest_attempt
            .output_schema
            .as_ref()
            .is_some_and(|output_schema| output_schema.serialized_bytes > 0)
    );

    let (hard_capped, hard_capped_wire_response) =
        inspect_context(&mut app_server, &thread.id, Some(u32::MAX)).await?;
    for private_text in [
        PRIVATE_USER_TEXT,
        PRIVATE_BASE_INSTRUCTIONS,
        PRIVATE_TOOL_SCHEMA_TEXT,
        PRIVATE_OUTPUT_SCHEMA_TEXT,
    ] {
        assert!(!hard_capped_wire_response.contains(private_text));
    }
    assert!(hard_capped.snapshot.applied_limits.max_items < u64::from(u32::MAX));
    assert_eq!(
        (
            &hard_capped.snapshot.raw.fingerprint,
            hard_capped.snapshot.raw.total_items,
            hard_capped.snapshot.raw.total_serialized_bytes,
            &hard_capped.snapshot.normalized.fingerprint,
            hard_capped.snapshot.normalized.total_items,
            hard_capped.snapshot.normalized.total_serialized_bytes,
            &hard_capped.snapshot.normalization,
        ),
        (
            &first.snapshot.raw.fingerprint,
            first.snapshot.raw.total_items,
            first.snapshot.raw.total_serialized_bytes,
            &first.snapshot.normalized.fingerprint,
            first.snapshot.normalized.total_items,
            first.snapshot.normalized.total_serialized_bytes,
            &first.snapshot.normalization,
        )
    );
    assert_eq!(
        hard_capped.snapshot.raw.represented_items,
        hard_capped.snapshot.raw.total_items
    );
    assert_eq!(
        hard_capped.snapshot.normalized.represented_items,
        hard_capped.snapshot.normalized.total_items
    );

    Ok(())
}

async fn inspect_context(
    app_server: &mut TestAppServer,
    thread_id: &str,
    max_items: Option<u32>,
) -> Result<(ThreadContextInspectResponse, String)> {
    let params = serde_json::to_value(ThreadContextInspectParams {
        thread_id: thread_id.to_string(),
        max_items,
    })?;
    let request_id = app_server
        .send_raw_request("thread/contextInspect", Some(params))
        .await?;
    let response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        app_server.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    let wire_response = serde_json::to_string(&response)?;
    Ok((to_response(response)?, wire_response))
}

fn create_config_toml(codex_home: &Path, server_uri: &str) -> std::io::Result<()> {
    std::fs::write(
        codex_home.join("config.toml"),
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"
model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )
}
