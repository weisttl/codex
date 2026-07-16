use codex_protocol::models::ContentItem;
use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;

const SECRET: &str = "actual-request-secret-must-not-be-disclosed";

fn message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

#[test]
fn request_projection_is_metadata_only_and_component_aware() -> Result<(), ContextInspectionError> {
    let normalized_input = vec![message(SECRET)];
    let provider_input = vec![message(&format!("provider-{SECRET}"))];
    let tools = vec![json!({"type": "function", "description": SECRET})];
    let output_schema = json!({"description": SECRET});
    let serialized_request = serde_json::to_vec(&json!({"input": SECRET}))?;
    let snapshot = ModelRequestSnapshot::project(ModelRequestProjection {
        sequence: 3,
        captured_at: 11,
        transport: ModelRequestTransport::ResponsesHttp,
        transport_uses_delta: false,
        connection_reused: false,
        transport_input_items: provider_input.len(),
        model: "test-model",
        provider: "test-provider",
        serialized_request: &serialized_request,
        normalized_input: &normalized_input,
        provider_input: &provider_input,
        instructions: Some(SECRET),
        tools: Some(&tools),
        output_schema: Some(&output_schema),
    })?;

    assert_eq!(snapshot.status, ModelRequestAttemptStatus::Prepared);
    assert_eq!(
        snapshot.tools.as_ref().map(|tools| tools.total_items),
        Some(1)
    );
    assert!(snapshot.instructions.is_some());
    assert!(snapshot.output_schema.is_some());
    let serialized_snapshot = serde_json::to_string(&snapshot)?;
    assert!(!serialized_snapshot.contains(SECRET));
    assert!(!serialized_snapshot.contains("description"));
    Ok(())
}

#[test]
fn request_projection_is_hard_capped_then_can_be_limited_at_inspection_time()
-> Result<(), ContextInspectionError> {
    let items = (0..(HARD_MAX_ITEMS + 1))
        .map(|index| message(&format!("item-{index}")))
        .collect::<Vec<_>>();
    let serialized_request = serde_json::to_vec(&items)?;
    let snapshot = ModelRequestSnapshot::project(ModelRequestProjection {
        sequence: 1,
        captured_at: 1,
        transport: ModelRequestTransport::ResponsesWebsocket,
        transport_uses_delta: true,
        connection_reused: true,
        transport_input_items: 1,
        model: "model",
        provider: "provider",
        serialized_request: &serialized_request,
        normalized_input: &items,
        provider_input: &items,
        instructions: None,
        tools: None,
        output_schema: None,
    })?;
    assert_eq!(snapshot.normalized_input.items.len(), HARD_MAX_ITEMS);

    let inspection = ModelRequestInspection {
        latest_attempt: Some(snapshot.clone()),
        last_actual: Some(snapshot),
        current_normalized_matches_last_actual: Some(true),
    }
    .with_limits(ContextInspectionLimits::with_max_items(
        /*max_items*/ 2,
    ));
    let expected = (2, to_u64(HARD_MAX_ITEMS - 1));
    assert_eq!(
        inspection.latest_attempt.as_ref().map(|snapshot| (
            snapshot.normalized_input.items.len(),
            snapshot.normalized_input.omitted_items,
        )),
        Some(expected)
    );
    assert_eq!(
        inspection.last_actual.as_ref().map(|snapshot| (
            snapshot.provider_input.items.len(),
            snapshot.provider_input.omitted_items,
        )),
        Some(expected)
    );
    Ok(())
}

#[test]
fn request_identity_changes_with_content_and_order() -> Result<(), ContextInspectionError> {
    let first = vec![message("first"), message("second")];
    let reversed = vec![message("second"), message("first")];
    let first_bytes = serde_json::to_vec(&first)?;
    let reversed_bytes = serde_json::to_vec(&reversed)?;

    let project = |items: &[ResponseItem], bytes: &[u8]| {
        ModelRequestSnapshot::project(ModelRequestProjection {
            sequence: 1,
            captured_at: 1,
            transport: ModelRequestTransport::ResponsesHttp,
            transport_uses_delta: false,
            connection_reused: false,
            transport_input_items: items.len(),
            model: "model",
            provider: "provider",
            serialized_request: bytes,
            normalized_input: items,
            provider_input: items,
            instructions: None,
            tools: None,
            output_schema: None,
        })
    };
    let first_snapshot = project(&first, &first_bytes)?;
    let reversed_snapshot = project(&reversed, &reversed_bytes)?;

    assert_ne!(first_snapshot.fingerprint, reversed_snapshot.fingerprint);
    assert_ne!(
        first_snapshot.normalized_input.fingerprint,
        reversed_snapshot.normalized_input.fingerprint
    );
    Ok(())
}

#[test]
fn model_and_provider_are_utf8_bounded() -> Result<(), ContextInspectionError> {
    let long_value = "界".repeat(super::super::MAX_METADATA_TEXT_BYTES);
    let serialized_request = serde_json::to_vec(&json!({}))?;
    let snapshot = ModelRequestSnapshot::project(ModelRequestProjection {
        sequence: 1,
        captured_at: 1,
        transport: ModelRequestTransport::ResponsesHttp,
        transport_uses_delta: false,
        connection_reused: false,
        transport_input_items: 0,
        model: &long_value,
        provider: &long_value,
        serialized_request: &serialized_request,
        normalized_input: &[],
        provider_input: &[],
        instructions: None,
        tools: None,
        output_schema: None,
    })?;

    assert!(snapshot.model.len() <= super::super::MAX_METADATA_TEXT_BYTES);
    assert!(snapshot.provider.len() <= super::super::MAX_METADATA_TEXT_BYTES);
    assert!(snapshot.model.is_char_boundary(snapshot.model.len()));
    assert!(snapshot.provider.is_char_boundary(snapshot.provider.len()));
    Ok(())
}
