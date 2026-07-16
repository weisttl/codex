use std::sync::Mutex;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use codex_api::ResponsesApiRequest;
use codex_context_inspector::ContextInspectionLimits;
use codex_context_inspector::ModelRequestAttemptStatus;
use codex_context_inspector::ModelRequestInspection;
use codex_context_inspector::ModelRequestProjection;
use codex_context_inspector::ModelRequestSnapshot;
use codex_context_inspector::ModelRequestTransport;
use codex_protocol::models::ResponseItem;
use serde_json::Value;
use tracing::debug;

#[derive(Debug)]
pub(crate) struct ModelRequestAttempt(ModelRequestSnapshot);

pub(crate) struct PreparedModelRequest<'a> {
    pub(crate) request: &'a ResponsesApiRequest,
    pub(crate) normalized_input: &'a [ResponseItem],
    pub(crate) output_schema: Option<&'a Value>,
    pub(crate) provider: &'a str,
    pub(crate) transport: ModelRequestTransport,
    pub(crate) transport_uses_delta: bool,
    pub(crate) connection_reused: bool,
    pub(crate) transport_input_items: usize,
}

#[derive(Debug, Default)]
pub(crate) struct ModelRequestObserver {
    state: Mutex<ModelRequestObserverState>,
}

#[derive(Debug, Default)]
struct ModelRequestObserverState {
    sequence: u64,
    latest_attempt: Option<ModelRequestSnapshot>,
    last_actual: Option<ModelRequestSnapshot>,
}

impl ModelRequestObserver {
    pub(crate) fn record_prepared(
        &self,
        prepared: PreparedModelRequest<'_>,
    ) -> Option<ModelRequestAttempt> {
        let serialized_request = match serde_json::to_vec(prepared.request) {
            Ok(serialized_request) => serialized_request,
            Err(err) => {
                debug!(%err, "failed to serialize model request for context inspection");
                return None;
            }
        };
        let sequence = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.sequence = state.sequence.saturating_add(1);
            state.sequence
        };
        let request = prepared.request;
        let snapshot = match ModelRequestSnapshot::project(ModelRequestProjection {
            sequence,
            captured_at: unix_timestamp_seconds(),
            transport: prepared.transport,
            transport_uses_delta: prepared.transport_uses_delta,
            connection_reused: prepared.connection_reused,
            transport_input_items: prepared.transport_input_items,
            model: &request.model,
            provider: prepared.provider,
            serialized_request: &serialized_request,
            normalized_input: prepared.normalized_input,
            provider_input: &request.input,
            instructions: (!request.instructions.is_empty()).then_some(&request.instructions),
            tools: request.tools.as_deref(),
            output_schema: prepared.output_schema,
        }) {
            Ok(snapshot) => snapshot,
            Err(err) => {
                debug!(%err, "failed to project model request for context inspection");
                return None;
            }
        };

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .latest_attempt
            .as_ref()
            .is_none_or(|latest| latest.sequence < sequence)
        {
            state.latest_attempt = Some(snapshot.clone());
        }
        Some(ModelRequestAttempt(snapshot))
    }

    pub(crate) fn record_stream_opened(&self, attempt: Option<ModelRequestAttempt>) {
        self.record_status(attempt, ModelRequestAttemptStatus::StreamOpened);
    }

    pub(crate) fn record_failed(&self, attempt: Option<ModelRequestAttempt>) {
        self.record_status(attempt, ModelRequestAttemptStatus::Failed);
    }

    pub(crate) fn inspect(&self, limits: ContextInspectionLimits) -> ModelRequestInspection {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ModelRequestInspection {
            latest_attempt: state.latest_attempt.clone(),
            last_actual: state.last_actual.clone(),
            current_normalized_matches_last_actual: None,
        }
        .with_limits(limits)
    }

    fn record_status(
        &self,
        attempt: Option<ModelRequestAttempt>,
        status: ModelRequestAttemptStatus,
    ) {
        let Some(ModelRequestAttempt(mut snapshot)) = attempt else {
            return;
        };
        snapshot.status = status;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .latest_attempt
            .as_ref()
            .is_some_and(|latest| latest.sequence == snapshot.sequence)
        {
            state.latest_attempt = Some(snapshot.clone());
        }
        if status == ModelRequestAttemptStatus::StreamOpened
            && state
                .last_actual
                .as_ref()
                .is_none_or(|actual| actual.sequence < snapshot.sequence)
        {
            state.last_actual = Some(snapshot);
        }
    }
}

fn unix_timestamp_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "request_observer_tests.rs"]
mod tests;
