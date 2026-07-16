use codex_app_server_protocol::ContextContentDisclosure;
use codex_app_server_protocol::ContextInspectionCollection;
use codex_app_server_protocol::ContextInspectionItem;
use codex_app_server_protocol::ContextInspectionLimits;
use codex_app_server_protocol::ContextInspectionSnapshot;
use codex_app_server_protocol::ContextNormalizationSummary;
use codex_app_server_protocol::ModelRequestAttemptStatus;
use codex_app_server_protocol::ModelRequestComponentSummary;
use codex_app_server_protocol::ModelRequestInspection;
use codex_app_server_protocol::ModelRequestSnapshot;
use codex_app_server_protocol::ModelRequestTransport;
use codex_app_server_protocol::ModelRequestValueCollectionSummary;
use ratatui::text::Line;

use super::render_error;
use super::render_snapshot;

#[test]
fn rich_context_snapshot() {
    let raw = collection(
        "sha256:raw",
        vec![("message", Some("user"), 120), ("reasoning", None, 80)],
    );
    let normalized = collection("sha256:normalized", vec![("message", Some("user"), 120)]);
    let sent = request(
        /*sequence*/ 4,
        ModelRequestAttemptStatus::Sent,
        ModelRequestTransport::ResponsesHttp,
        normalized.clone(),
        normalized.clone(),
    );
    let failed = request(
        /*sequence*/ 5,
        ModelRequestAttemptStatus::Failed,
        ModelRequestTransport::ResponsesWebsocket,
        normalized.clone(),
        collection(
            "sha256:provider",
            vec![
                ("message", Some("developer"), 64),
                ("message", Some("user"), 120),
            ],
        ),
    );
    let snapshot = ContextInspectionSnapshot {
        history_version: 7,
        content_disclosure: ContextContentDisclosure::MetadataOnly,
        applied_limits: ContextInspectionLimits { max_items: 128 },
        raw,
        normalized,
        normalization: ContextNormalizationSummary {
            unchanged_items: 1,
            raw_only_items: 1,
            normalized_only_items: 0,
            changed: true,
        },
        request: ModelRequestInspection {
            latest_attempt: Some(failed),
            last_actual: Some(sent),
            current_normalized_matches_last_actual: Some(true),
        },
    };

    insta::assert_snapshot!(plain(render_snapshot(&snapshot)));
}

#[test]
fn empty_context_snapshot() {
    let empty = collection("sha256:empty", Vec::new());
    let snapshot = ContextInspectionSnapshot {
        history_version: 0,
        content_disclosure: ContextContentDisclosure::MetadataOnly,
        applied_limits: ContextInspectionLimits { max_items: 128 },
        raw: empty.clone(),
        normalized: empty,
        normalization: ContextNormalizationSummary {
            unchanged_items: 0,
            raw_only_items: 0,
            normalized_only_items: 0,
            changed: false,
        },
        request: ModelRequestInspection {
            latest_attempt: None,
            last_actual: None,
            current_normalized_matches_last_actual: None,
        },
    };

    insta::assert_snapshot!(plain(render_snapshot(&snapshot)));
}

#[test]
fn context_inspection_error() {
    insta::assert_snapshot!(plain(render_error("thread not found")));
}

fn collection(
    fingerprint: &str,
    items: Vec<(&str, Option<&str>, u64)>,
) -> ContextInspectionCollection {
    let total_serialized_bytes = items.iter().map(|(_, _, bytes)| bytes).sum();
    ContextInspectionCollection {
        fingerprint: fingerprint.to_string(),
        total_items: items.len() as u64,
        represented_items: items.len() as u64,
        omitted_items: 0,
        total_serialized_bytes,
        items: items
            .into_iter()
            .enumerate()
            .map(
                |(index, (kind, role, serialized_bytes))| ContextInspectionItem {
                    index: index as u64,
                    kind: kind.to_string(),
                    role: role.map(str::to_string),
                    serialized_bytes,
                },
            )
            .collect(),
    }
}

fn request(
    sequence: u64,
    status: ModelRequestAttemptStatus,
    transport: ModelRequestTransport,
    normalized_input: ContextInspectionCollection,
    provider_input: ContextInspectionCollection,
) -> ModelRequestSnapshot {
    ModelRequestSnapshot {
        sequence,
        captured_at: 1_700_000_000,
        status,
        transport,
        transport_uses_delta: transport == ModelRequestTransport::ResponsesWebsocket,
        connection_reused: transport == ModelRequestTransport::ResponsesWebsocket,
        transport_input_items: 1,
        model: "gpt-test".to_string(),
        provider: "OpenAI".to_string(),
        fingerprint: format!("sha256:request-{sequence}"),
        serialized_bytes: 2_048,
        normalized_input,
        provider_input,
        instructions: Some(ModelRequestComponentSummary {
            fingerprint: "sha256:instructions".to_string(),
            serialized_bytes: 512,
        }),
        tools: Some(ModelRequestValueCollectionSummary {
            fingerprint: "sha256:tools".to_string(),
            total_items: 3,
            serialized_bytes: 768,
        }),
        output_schema: None,
    }
}

fn plain(lines: Vec<Line<'static>>) -> String {
    lines
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
