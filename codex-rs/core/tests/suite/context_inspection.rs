use codex_core::ContextInspectionLimits;
use codex_core::CurrentContextSnapshot;
use codex_core::ModelRequestAttemptStatus;
use codex_core::ModelRequestTransport;
use codex_protocol::models::ResponseItem;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_response_once;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::responses::start_websocket_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;
use wiremock::ResponseTemplate;

const PRIVATE_USER_TEXT: &str = "context-inspection-private-user-text";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inspection_is_repeatable_and_matches_the_next_request_prefix() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let first_request = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("response-1"),
            ev_completed("response-1"),
        ]),
    )
    .await;
    let second_request = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("response-2"),
            ev_completed("response-2"),
        ]),
    )
    .await;
    let test = test_codex().build_with_auto_env(&server).await?;

    let before_turn = test
        .codex
        .inspect_current_context(ContextInspectionLimits::default())
        .await?;
    assert_eq!(before_turn.request.latest_attempt, None);
    assert_eq!(before_turn.request.last_actual, None);
    assert_eq!(
        before_turn.request.current_normalized_matches_last_actual,
        None
    );

    test.submit_turn(PRIVATE_USER_TEXT).await?;
    let first_request = first_request.single_request();
    let first_request_input = first_request
        .input()
        .into_iter()
        .map(serde_json::from_value)
        .collect::<serde_json::Result<Vec<ResponseItem>>>()?;

    let inspected = test
        .codex
        .inspect_current_context(ContextInspectionLimits::default())
        .await?;
    let repeated = test
        .codex
        .inspect_current_context(ContextInspectionLimits::default())
        .await?;
    assert_eq!(inspected, repeated);
    let latest_attempt = inspected
        .request
        .latest_attempt
        .as_ref()
        .expect("latest request attempt");
    let last_actual = inspected
        .request
        .last_actual
        .as_ref()
        .expect("last actual request");
    assert_eq!(latest_attempt, last_actual);
    assert_eq!(latest_attempt.status, ModelRequestAttemptStatus::Sent);
    assert_eq!(
        latest_attempt.transport,
        ModelRequestTransport::ResponsesHttp
    );
    assert!(!latest_attempt.transport_uses_delta);
    assert!(!latest_attempt.connection_reused);
    assert_eq!(
        latest_attempt.transport_input_items,
        u64::try_from(first_request_input.len())?
    );
    let outbound_projection = CurrentContextSnapshot::project(
        /*history_version*/ 0,
        &first_request_input,
        &first_request_input,
        ContextInspectionLimits::with_max_items(/*max_items*/ 0),
    )?;
    assert_eq!(
        last_actual.normalized_input.fingerprint,
        inspected.normalized.fingerprint
    );
    assert_eq!(
        last_actual.provider_input.fingerprint,
        outbound_projection.normalized.fingerprint
    );
    assert_eq!(
        inspected.request.current_normalized_matches_last_actual,
        Some(true)
    );
    let serialized_inspection = serde_json::to_string(&inspected.request)?;
    assert!(!serialized_inspection.contains(PRIVATE_USER_TEXT));

    let zero_limited = test
        .codex
        .inspect_current_context(ContextInspectionLimits::with_max_items(
            /*max_items*/ 0,
        ))
        .await?;
    let zero_limited_actual = zero_limited
        .request
        .last_actual
        .as_ref()
        .expect("zero-limited actual request");
    assert_eq!(zero_limited_actual.normalized_input.items, Vec::new());
    assert_eq!(zero_limited_actual.provider_input.items, Vec::new());
    assert!(zero_limited_actual.normalized_input.total_items > 0);
    assert!(zero_limited_actual.provider_input.total_items > 0);

    test.submit_turn("second turn").await?;
    let request_input = second_request
        .single_request()
        .input()
        .into_iter()
        .map(serde_json::from_value)
        .collect::<serde_json::Result<Vec<ResponseItem>>>()?;
    let inspected_items = usize::try_from(inspected.normalized.total_items)?;
    assert_eq!(request_input.len(), inspected_items + 1);

    let request_prefix = &request_input[..inspected_items];
    let prefix_projection = CurrentContextSnapshot::project(
        /*history_version*/ 0,
        request_prefix,
        request_prefix,
        ContextInspectionLimits::with_max_items(/*max_items*/ 0),
    )?;
    assert_eq!(
        inspected.normalized.fingerprint,
        prefix_projection.normalized.fingerprint
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_attempt_does_not_replace_last_actual_request() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("response-1"),
            ev_completed("response-1"),
        ]),
    )
    .await;
    mount_response_once(
        &server,
        ResponseTemplate::new(/*status*/ 500).set_body_json(serde_json::json!({
            "error": {
                "type": "server_error",
                "message": "synthetic request failure",
            }
        })),
    )
    .await;
    let mut builder = test_codex().with_config(|config| {
        config.model_provider.request_max_retries = Some(0);
    });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("successful turn").await?;
    let after_success = test
        .codex
        .inspect_current_context(ContextInspectionLimits::default())
        .await?;
    let successful_request = after_success
        .request
        .last_actual
        .expect("successful actual request");

    test.submit_turn("failed turn").await?;
    let after_failure = test
        .codex
        .inspect_current_context(ContextInspectionLimits::default())
        .await?;
    let failed_attempt = after_failure
        .request
        .latest_attempt
        .expect("failed request attempt");

    assert!(failed_attempt.sequence > successful_request.sequence);
    assert_eq!(failed_attempt.status, ModelRequestAttemptStatus::Failed);
    assert_eq!(after_failure.request.last_actual, Some(successful_request));
    assert_eq!(
        after_failure.request.current_normalized_matches_last_actual,
        Some(false)
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_inspection_keeps_full_logical_input_separate_from_transport_delta()
-> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_websocket_server(vec![vec![
        vec![ev_response_created("warm-1"), ev_completed("warm-1")],
        vec![
            ev_response_created("response-1"),
            ev_assistant_message("message-1", "assistant output"),
            ev_completed("response-1"),
        ],
        vec![
            ev_response_created("response-2"),
            ev_completed("response-2"),
        ],
    ]])
    .await;
    let mut builder = test_codex();
    let test = builder.build_with_websocket_server(&server).await?;

    let before_turn = test
        .codex
        .inspect_current_context(ContextInspectionLimits::default())
        .await?;
    assert_eq!(before_turn.request.latest_attempt, None);
    assert_eq!(before_turn.request.last_actual, None);

    test.submit_turn("first websocket turn").await?;
    let after_first = test
        .codex
        .inspect_current_context(ContextInspectionLimits::default())
        .await?;
    assert_eq!(
        after_first
            .request
            .last_actual
            .as_ref()
            .map(|request| request.sequence),
        Some(1)
    );

    test.submit_turn("second websocket turn").await?;
    let inspected = test
        .codex
        .inspect_current_context(ContextInspectionLimits::default())
        .await?;
    let latest_attempt = inspected
        .request
        .latest_attempt
        .as_ref()
        .expect("latest websocket attempt");
    let last_actual = inspected
        .request
        .last_actual
        .as_ref()
        .expect("last actual websocket request");
    assert_eq!(latest_attempt, last_actual);
    assert_eq!(last_actual.sequence, 2);
    assert_eq!(last_actual.status, ModelRequestAttemptStatus::Sent);
    assert_eq!(
        last_actual.transport,
        ModelRequestTransport::ResponsesWebsocket
    );
    assert!(last_actual.transport_uses_delta);
    assert!(last_actual.connection_reused);

    let connection = server.single_connection();
    assert_eq!(connection.len(), 3);
    let transport_input_items = connection
        .get(2)
        .and_then(|request| request.body_json()["input"].as_array().map(Vec::len))
        .expect("second turn transport input");
    assert_eq!(
        last_actual.transport_input_items,
        u64::try_from(transport_input_items)?
    );
    assert!(last_actual.normalized_input.total_items > last_actual.transport_input_items);
    assert!(last_actual.provider_input.total_items > last_actual.transport_input_items);
    assert_eq!(
        inspected.request.current_normalized_matches_last_actual,
        Some(true)
    );

    server.shutdown().await;
    Ok(())
}
