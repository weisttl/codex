use std::io::Write;

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

const REQUEST_FINGERPRINT_VERSION: &[u8] = b"codex-model-request-v2\0";
const COMPONENT_FINGERPRINT_VERSION: &[u8] = b"codex-model-request-component-v2\0";

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
    Sent,
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
pub struct ModelRequestProjection<'a, T: Serialize + ?Sized> {
    pub sequence: u64,
    pub captured_at: i64,
    pub transport: ModelRequestTransport,
    pub transport_uses_delta: bool,
    pub connection_reused: bool,
    pub transport_input_items: usize,
    pub model: &'a str,
    pub provider: &'a str,
    pub request: &'a T,
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
    pub fn project<T: Serialize + ?Sized>(
        projection: ModelRequestProjection<'_, T>,
    ) -> Result<Self, ContextInspectionError> {
        let storage_limits = ContextInspectionLimits::with_max_items(HARD_MAX_ITEMS);
        let normalized_input = project_collection(projection.normalized_input, storage_limits)?;
        let provider_input = if projection.normalized_input == projection.provider_input {
            normalized_input.collection.clone()
        } else {
            project_collection(projection.provider_input, storage_limits)?.collection
        };
        let request = serialized_summary(REQUEST_FINGERPRINT_VERSION, projection.request)?;
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
            fingerprint: request.fingerprint,
            serialized_bytes: request.serialized_bytes,
            normalized_input: normalized_input.collection,
            provider_input,
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

/// Session-scoped observations for the latest attempt and latest sent request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequestInspection {
    pub latest_attempt: Option<ModelRequestSnapshot>,
    pub last_actual: Option<ModelRequestSnapshot>,
    /// `None` until a request has been sent successfully at the transport boundary.
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
    serialized_summary(COMPONENT_FINGERPRINT_VERSION, value)
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

fn serialized_summary<T: Serialize + ?Sized>(
    version: &[u8],
    value: &T,
) -> Result<ModelRequestComponentSummary, serde_json::Error> {
    let mut writer = FingerprintWriter::new(version);
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.finish())
}

struct FingerprintWriter {
    hasher: Sha256,
    serialized_bytes: u64,
}

impl FingerprintWriter {
    fn new(version: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(version);
        Self {
            hasher,
            serialized_bytes: 0,
        }
    }

    fn finish(mut self) -> ModelRequestComponentSummary {
        self.hasher.update(self.serialized_bytes.to_be_bytes());
        ModelRequestComponentSummary {
            fingerprint: format_fingerprint(self.hasher.finalize().into()),
            serialized_bytes: self.serialized_bytes,
        }
    }
}

impl Write for FingerprintWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buf);
        self.serialized_bytes = self.serialized_bytes.saturating_add(to_u64(buf.len()));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
