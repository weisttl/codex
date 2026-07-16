use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadContextInspectParams {
    pub thread_id: String,
    /// Maximum number of item metadata records to return for each collection.
    /// The server clamps this value to its hard limit.
    #[ts(optional = nullable)]
    pub max_items: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadContextInspectResponse {
    pub snapshot: ContextInspectionSnapshot,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ContextInspectionSnapshot {
    /// History rewrite generation from the live context manager.
    pub history_version: u64,
    pub content_disclosure: ContextContentDisclosure,
    pub applied_limits: ContextInspectionLimits,
    /// Current history before production prompt normalization.
    pub raw: ContextInspectionCollection,
    /// Current history after production prompt normalization.
    pub normalized: ContextInspectionCollection,
    pub normalization: ContextNormalizationSummary,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ContextInspectionLimits {
    pub max_items: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ContextInspectionCollection {
    /// Versioned SHA-256 fingerprint of all complete serialized items in order.
    pub fingerprint: String,
    pub total_items: u64,
    pub represented_items: u64,
    pub omitted_items: u64,
    /// Sum of complete serialized item sizes, including undisclosed content.
    pub total_serialized_bytes: u64,
    pub items: Vec<ContextInspectionItem>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ContextInspectionItem {
    pub index: u64,
    /// Bounded serialized response item type tag.
    pub kind: String,
    /// Bounded serialized role when the item has one.
    pub role: Option<String>,
    /// Size of the complete serialized item, including undisclosed content.
    pub serialized_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ContextNormalizationSummary {
    /// Items with identical complete serialization in both collections, counted as a multiset.
    pub unchanged_items: u64,
    pub raw_only_items: u64,
    pub normalized_only_items: u64,
    /// Whether normalization changed item identity, count, or order.
    pub changed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub enum ContextContentDisclosure {
    /// Only bounded metadata is disclosed; content, arguments, outputs, schemas, and IDs are not.
    MetadataOnly,
}
