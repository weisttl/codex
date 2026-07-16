use codex_context_inspector::ModelRequestProjection;
use pretty_assertions::assert_eq;

use super::*;

#[test]
fn overlapping_attempts_keep_latest_status_and_newest_actual_separate() {
    let observer = ModelRequestObserver::default();
    let first = snapshot(/*sequence*/ 1);
    let second = snapshot(/*sequence*/ 2);
    observer
        .state
        .lock()
        .expect("observer state lock")
        .latest_attempt = Some(second.clone());

    observer.record_sent(Some(ModelRequestAttempt(first.clone())));
    let mut sent_first = first;
    sent_first.status = ModelRequestAttemptStatus::Sent;
    assert_eq!(
        observer.inspect(ContextInspectionLimits::default()),
        ModelRequestInspection {
            latest_attempt: Some(second.clone()),
            last_actual: Some(sent_first.clone()),
            current_normalized_matches_last_actual: None,
        }
    );

    observer.record_failed(Some(ModelRequestAttempt(second.clone())));
    let mut failed_second = second;
    failed_second.status = ModelRequestAttemptStatus::Failed;
    assert_eq!(
        observer.inspect(ContextInspectionLimits::default()),
        ModelRequestInspection {
            latest_attempt: Some(failed_second),
            last_actual: Some(sent_first),
            current_normalized_matches_last_actual: None,
        }
    );
}

fn snapshot(sequence: u64) -> ModelRequestSnapshot {
    let request = serde_json::json!({
        "sequence": sequence,
    });
    ModelRequestSnapshot::project(ModelRequestProjection {
        sequence,
        captured_at: 1,
        transport: ModelRequestTransport::ResponsesHttp,
        transport_uses_delta: false,
        connection_reused: false,
        transport_input_items: 0,
        model: "model",
        provider: "provider",
        request: &request,
        normalized_input: &[],
        provider_input: &[],
        instructions: None,
        tools: None,
        output_schema: None,
    })
    .expect("project request fixture")
}
