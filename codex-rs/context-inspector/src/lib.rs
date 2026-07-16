//! Bounded, metadata-only projections of Codex model-visible history.
//!
//! This crate deliberately has no dependency on `codex-core`. Callers supply both the raw
//! history and the normalized history produced by the same path used for model requests.

use std::collections::HashMap;
use std::io::Write;

use codex_protocol::models::ResponseItem;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use thiserror::Error;

mod request;

pub use request::ModelRequestAttemptStatus;
pub use request::ModelRequestComponentSummary;
pub use request::ModelRequestInspection;
pub use request::ModelRequestProjection;
pub use request::ModelRequestSnapshot;
pub use request::ModelRequestTransport;
pub use request::ModelRequestValueCollectionSummary;

/// Default number of item metadata records represented in each collection.
pub const DEFAULT_MAX_ITEMS: usize = 128;
/// Absolute maximum number of item metadata records represented in each collection.
pub const HARD_MAX_ITEMS: usize = 512;

const MAX_METADATA_TEXT_BYTES: usize = 128;
const COLLECTION_FINGERPRINT_VERSION: &[u8] = b"codex-context-items-v2\0";

/// Limits applied while projecting a context snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextInspectionLimits {
    max_items: usize,
}

impl ContextInspectionLimits {
    /// Creates limits with `max_items` clamped to [`HARD_MAX_ITEMS`].
    pub fn with_max_items(max_items: usize) -> Self {
        Self {
            max_items: max_items.min(HARD_MAX_ITEMS),
        }
    }

    /// Returns the applied item metadata limit.
    pub fn max_items(self) -> usize {
        self.max_items
    }
}

impl Default for ContextInspectionLimits {
    fn default() -> Self {
        Self::with_max_items(DEFAULT_MAX_ITEMS)
    }
}

/// Describes whether model-visible content is included in a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentDisclosure {
    /// Only bounded metadata is disclosed; content, arguments, outputs, schemas, and IDs are not.
    MetadataOnly,
}

/// Metadata for one response item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextInspectionItem {
    /// Zero-based position in the inspected collection.
    pub index: u64,
    /// Bounded serialized `type` tag.
    pub kind: String,
    /// Bounded serialized role, when the item has one.
    pub role: Option<String>,
    /// Size of the complete serialized item, including content that is not disclosed.
    pub serialized_bytes: u64,
}

/// A bounded projection and complete aggregate statistics for one item collection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextItemCollection {
    /// Versioned SHA-256 fingerprint of all complete serialized items in order.
    pub fingerprint: String,
    /// Total number of items in the collection.
    pub total_items: u64,
    /// Number of item metadata records returned.
    pub represented_items: u64,
    /// Number of item metadata records omitted by the applied limit.
    pub omitted_items: u64,
    /// Sum of complete serialized item sizes.
    pub total_serialized_bytes: u64,
    /// Bounded item metadata records.
    pub items: Vec<ContextInspectionItem>,
}

impl ContextItemCollection {
    fn apply_limit(&mut self, limits: ContextInspectionLimits) {
        self.items.truncate(limits.max_items());
        self.represented_items = to_u64(self.items.len());
        self.omitted_items = self.total_items.saturating_sub(self.represented_items);
    }
}

/// Content-identity comparison between raw and normalized history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextNormalizationSummary {
    /// Items with identical complete serialization in both collections, counted as a multiset.
    pub unchanged_items: u64,
    /// Raw item identities that do not occur in normalized history.
    pub raw_only_items: u64,
    /// Normalized item identities that do not occur in raw history.
    pub normalized_only_items: u64,
    /// Whether normalization changed item identity, count, or order.
    pub changed: bool,
}

/// A point-in-time projection of current raw and normalized history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentContextSnapshot {
    /// History rewrite generation from the source context manager.
    pub history_version: u64,
    /// Disclosure policy used for this projection.
    pub content_disclosure: ContentDisclosure,
    /// Limits after hard-cap enforcement.
    pub applied_limits: ContextInspectionLimits,
    /// Current history before prompt normalization.
    pub raw: ContextItemCollection,
    /// Current history after prompt normalization.
    pub normalized: ContextItemCollection,
    /// Identity-level summary of normalization effects.
    pub normalization: ContextNormalizationSummary,
    /// Latest prepared request attempt and latest request sent at the transport boundary.
    pub request: ModelRequestInspection,
}

impl CurrentContextSnapshot {
    /// Projects raw and normalized response items into a bounded, metadata-only snapshot.
    pub fn project(
        history_version: u64,
        raw_items: &[ResponseItem],
        normalized_items: &[ResponseItem],
        limits: ContextInspectionLimits,
    ) -> Result<Self, ContextInspectionError> {
        let raw = project_collection(raw_items, limits)?;
        let normalized = project_collection(normalized_items, limits)?;
        let normalization =
            summarize_normalization(&raw.item_fingerprints, &normalized.item_fingerprints);

        Ok(Self {
            history_version,
            content_disclosure: ContentDisclosure::MetadataOnly,
            applied_limits: limits,
            raw: raw.collection,
            normalized: normalized.collection,
            normalization,
            request: ModelRequestInspection::default(),
        })
    }

    /// Attaches the session's request observations and compares current normalized history with the
    /// latest request that was sent successfully at the transport boundary.
    pub fn with_request_inspection(mut self, mut request: ModelRequestInspection) -> Self {
        request.current_normalized_matches_last_actual = request
            .last_actual
            .as_ref()
            .map(|actual| actual.normalized_input.fingerprint == self.normalized.fingerprint);
        self.request = request;
        self
    }
}

/// Failure to create a context projection.
#[derive(Debug, Error)]
pub enum ContextInspectionError {
    /// A response item could not be serialized as metadata.
    #[error("failed to serialize context inspection metadata: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub(crate) struct CollectionProjection {
    pub(crate) collection: ContextItemCollection,
    pub(crate) item_fingerprints: Vec<[u8; 32]>,
}

pub(crate) fn project_collection(
    items: &[ResponseItem],
    limits: ContextInspectionLimits,
) -> Result<CollectionProjection, ContextInspectionError> {
    let represented_capacity = items.len().min(limits.max_items());
    let mut represented = Vec::with_capacity(represented_capacity);
    let mut item_fingerprints = Vec::with_capacity(items.len());
    let mut total_serialized_bytes = 0_u64;
    let mut collection_hasher = Sha256::new();
    collection_hasher.update(COLLECTION_FINGERPRINT_VERSION);

    for (index, item) in items.iter().enumerate() {
        let mut writer = ItemFingerprintWriter::default();
        serde_json::to_writer(&mut writer, item)?;
        let (serialized_bytes, item_fingerprint) = writer.finish();
        total_serialized_bytes = total_serialized_bytes.saturating_add(serialized_bytes);
        collection_hasher.update(serialized_bytes.to_be_bytes());
        collection_hasher.update(item_fingerprint);
        item_fingerprints.push(item_fingerprint);

        if represented.len() < limits.max_items() {
            let (kind, role) = item_kind_and_role(item);
            represented.push(ContextInspectionItem {
                index: to_u64(index),
                kind: kind.to_string(),
                role: role.map(bounded_metadata_text),
                serialized_bytes,
            });
        }
    }

    let total_items = to_u64(items.len());
    let represented_items = to_u64(represented.len());
    Ok(CollectionProjection {
        collection: ContextItemCollection {
            fingerprint: format_fingerprint(collection_hasher.finalize().into()),
            total_items,
            represented_items,
            omitted_items: total_items.saturating_sub(represented_items),
            total_serialized_bytes,
            items: represented,
        },
        item_fingerprints,
    })
}

#[derive(Default)]
struct ItemFingerprintWriter {
    hasher: Sha256,
    serialized_bytes: u64,
}

impl ItemFingerprintWriter {
    fn finish(self) -> (u64, [u8; 32]) {
        (self.serialized_bytes, self.hasher.finalize().into())
    }
}

impl Write for ItemFingerprintWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buf);
        self.serialized_bytes = self.serialized_bytes.saturating_add(to_u64(buf.len()));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn item_kind_and_role(item: &ResponseItem) -> (&'static str, Option<&str>) {
    match item {
        ResponseItem::AdditionalTools { role, .. } => ("additional_tools", Some(role)),
        ResponseItem::Message { role, .. } => ("message", Some(role)),
        ResponseItem::AgentMessage { .. } => ("agent_message", None),
        ResponseItem::Reasoning { .. } => ("reasoning", None),
        ResponseItem::LocalShellCall { .. } => ("local_shell_call", None),
        ResponseItem::FunctionCall { .. } => ("function_call", None),
        ResponseItem::ToolSearchCall { .. } => ("tool_search_call", None),
        ResponseItem::FunctionCallOutput { .. } => ("function_call_output", None),
        ResponseItem::CustomToolCall { .. } => ("custom_tool_call", None),
        ResponseItem::CustomToolCallOutput { .. } => ("custom_tool_call_output", None),
        ResponseItem::ToolSearchOutput { .. } => ("tool_search_output", None),
        ResponseItem::WebSearchCall { .. } => ("web_search_call", None),
        ResponseItem::ImageGenerationCall { .. } => ("image_generation_call", None),
        ResponseItem::Compaction { .. } => ("compaction", None),
        ResponseItem::CompactionTrigger { .. } => ("compaction_trigger", None),
        ResponseItem::ContextCompaction { .. } => ("context_compaction", None),
        ResponseItem::Other => ("unknown", None),
    }
}

fn summarize_normalization(
    raw_fingerprints: &[[u8; 32]],
    normalized_fingerprints: &[[u8; 32]],
) -> ContextNormalizationSummary {
    let mut remaining_raw = HashMap::<[u8; 32], usize>::new();
    for fingerprint in raw_fingerprints {
        *remaining_raw.entry(*fingerprint).or_default() += 1;
    }

    let mut unchanged_items = 0_usize;
    for fingerprint in normalized_fingerprints {
        if let Some(remaining) = remaining_raw.get_mut(fingerprint)
            && *remaining > 0
        {
            *remaining -= 1;
            unchanged_items += 1;
        }
    }

    let raw_only_items = raw_fingerprints.len().saturating_sub(unchanged_items);
    let normalized_only_items = normalized_fingerprints
        .len()
        .saturating_sub(unchanged_items);
    ContextNormalizationSummary {
        unchanged_items: to_u64(unchanged_items),
        raw_only_items: to_u64(raw_only_items),
        normalized_only_items: to_u64(normalized_only_items),
        changed: raw_fingerprints != normalized_fingerprints,
    }
}

pub(crate) fn bounded_metadata_text(value: &str) -> String {
    let mut end = value.len().min(MAX_METADATA_TEXT_BYTES);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_string()
}

pub(crate) fn format_fingerprint(digest: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut fingerprint = String::with_capacity("sha256:".len() + digest.len() * 2);
    fingerprint.push_str("sha256:");
    for byte in digest {
        fingerprint.push(char::from(HEX[usize::from(byte >> 4)]));
        fingerprint.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    fingerprint
}

pub(crate) fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
