use codex_protocol::models::ResponseItem;
use serde::Serialize;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;

use super::ContextInspectionError;
use super::ContextInspectionLimits;
use super::HARD_MAX_ITEMS;
use super::bounded_metadata_text;
use super::format_fingerprint;
use super::project_collection;
use super::to_u64;
use crate::ContextItemCollection;

const REQUEST_FINGERPRINT_VERSION: &[u8] = b"codex-model-request-v1\0";
const COMPONENT_FINGERPRINT_VERSION: &[u8] = b"codex-model-request-component-v1\0";

/// Provider transport used for a model request attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelRequestTransport {
    ResponsesHttp,
    ResponsesWebsocket,
}

/// Furthest lifecycle point observed for a model request attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelRequestAttemptStatus {
    Prepared,
    StreamOpened,
    Failed,
}

/// Metadata-only identity and size for one request component.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequestComponentSummary {
    pub fingerprint: String,
    pub serialized_bytes: u64,
}

/// Metadata-only identity, size, and cardinality for a JSON value collection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequestValueCollectionSummary {
    pub fingerprint: String,
    pub total_items: u64,
    pub serialized_bytes: u64,
}

/// Inputs needed to project one complete logical provider request without retaining its contents.
pub struct ModelRequestProjection<'a> {
    pub sequence: u64,
    pub captured_at: i64,
    pub transport: ModelRequestTransport,
    pub transport_uses_delta: bool,
    pub connection_reused: bool,
    pub transport_input_items: usize,
    pub model: &'a str,
    pub provider: &'a str,
    pub serialized_request: &'a [u8],
    pub normalized_input: &'a [ResponseItem],
    pub provider_input: &'a [ResponseItem],
    pub instructions: Option<&'a str>,
    pub tools: Option<&'a [Value]>,
    pub output_schema: Option<&'a Value>,
}

/// Bounded, metadata-only projection of one complete logical provider request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequestSnapshot {
    pub sequence: u64,
    /// Unix timestamp in seconds when the request was prepared.
    pub captured_at: i64,
    pub status: ModelRequestAttemptStatus,
    pub transport: ModelRequestTransport,
    /// Whether the WebSocket transport sent only a delta with `previous_response_id`.
    pub transport_uses_delta: bool,
    /// Whether the WebSocket connection was reused. Always false for HTTP.
    pub connection_reused: bool,
    /// Number of input items present in the actual transport payload.
    pub transport_input_items: u64,
    pub model: String,
    pub provider: String,
    /// Identity of the complete logical provider request, not a WebSocket delta payload.
    pub fingerprint: String,
    pub serialized_bytes: u64,
    /// Production prompt input before provider-specific request adaptation.
    pub normalized_input: ContextItemCollection,
    /// Complete logical input after provider-specific request adaptation.
    pub provider_input: ContextItemCollection,
    pub instructions: Option<ModelRequestComponentSummary>,
    pub tools: Option<ModelRequestValueCollectionSummary>,
    pub output_schema: Option<ModelRequestComponentSummary>,
}

impl ModelRequestSnapshot {
    pub fn project(projection: ModelRequestProjection<'_>) -> Result<Self, ContextInspectionError> {
        let storage_limits = ContextInspectionLimits::with_max_items(HARD_MAX_ITEMS);
        Ok(Self {
            sequence: projection.sequence,
            captured_at: projection.captured_at,
            status: ModelRequestAttemptStatus::Prepared,
            transport: projection.transport,
            transport_uses_delta: projection.transport_uses_delta,
            connection_reused: projection.connection_reused,
            transport_input_items: to_u64(projection.transport_input_items),
            model: bounded_metadata_text(projection.model),
            provider: bounded_metadata_text(projection.provider),
            fingerprint: fingerprint_bytes(
                REQUEST_FINGERPRINT_VERSION,
                projection.serialized_request,
            ),
            serialized_bytes: to_u64(projection.serialized_request.len()),
            normalized_input: project_collection(projection.normalized_input, storage_limits)?
                .collection,
            provider_input: project_collection(projection.provider_input, storage_limits)?
                .collection,
            instructions: projection.instructions.map(component_summary).transpose()?,
            tools: projection.tools.map(value_collection_summary).transpose()?,
            output_schema: projection
                .output_schema
                .map(component_summary)
                .transpose()?,
        })
    }

    fn apply_limit(&mut self, limits: ContextInspectionLimits) {
        self.normalized_input.apply_limit(limits);
        self.provider_input.apply_limit(limits);
    }
}

/// Session-scoped observations for the latest attempt and latest opened response stream.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequestInspection {
    pub latest_attempt: Option<ModelRequestSnapshot>,
    pub last_actual: Option<ModelRequestSnapshot>,
    /// `None` until a request has successfully opened a provider response stream.
    pub current_normalized_matches_last_actual: Option<bool>,
}

impl ModelRequestInspection {
    pub fn with_limits(mut self, limits: ContextInspectionLimits) -> Self {
        if let Some(latest_attempt) = &mut self.latest_attempt {
            latest_attempt.apply_limit(limits);
        }
        if let Some(last_actual) = &mut self.last_actual {
            last_actual.apply_limit(limits);
        }
        self
    }
}

fn component_summary<T: Serialize + ?Sized>(
    value: &T,
) -> Result<ModelRequestComponentSummary, serde_json::Error> {
    let serialized = serde_json::to_vec(value)?;
    Ok(ModelRequestComponentSummary {
        fingerprint: fingerprint_bytes(COMPONENT_FINGERPRINT_VERSION, &serialized),
        serialized_bytes: to_u64(serialized.len()),
    })
}

fn value_collection_summary(
    values: &[Value],
) -> Result<ModelRequestValueCollectionSummary, serde_json::Error> {
    let summary = component_summary(values)?;
    Ok(ModelRequestValueCollectionSummary {
        fingerprint: summary.fingerprint,
        total_items: to_u64(values.len()),
        serialized_bytes: summary.serialized_bytes,
    })
}

fn fingerprint_bytes(version: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(version);
    hasher.update(to_u64(bytes.len()).to_be_bytes());
    hasher.update(bytes);
    format_fingerprint(hasher.finalize().into())
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
