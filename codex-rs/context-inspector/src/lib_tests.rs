use super::*;
use codex_protocol::models::ContentItem;
use pretty_assertions::assert_eq;

const SECRET_CONTENT: &str = "secret-context-content-must-not-be-disclosed";

fn message(role: &str, text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: role.to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

#[test]
fn projection_is_metadata_only() -> Result<(), ContextInspectionError> {
    let items = vec![message("user", SECRET_CONTENT)];
    let snapshot = CurrentContextSnapshot::project(
        /*history_version*/ 7,
        &items,
        &items,
        ContextInspectionLimits::default(),
    )?;

    assert_eq!(snapshot.history_version, 7);
    assert_eq!(snapshot.content_disclosure, ContentDisclosure::MetadataOnly);
    assert_eq!(
        snapshot.raw.items,
        vec![ContextInspectionItem {
            index: 0,
            kind: "message".to_string(),
            role: Some("user".to_string()),
            serialized_bytes: snapshot.raw.total_serialized_bytes,
        }]
    );
    let serialized_snapshot = serde_json::to_string(&snapshot)?;
    assert!(!serialized_snapshot.contains(SECRET_CONTENT));
    assert!(!serialized_snapshot.contains("\"content\":"));
    Ok(())
}

#[test]
fn item_limit_is_hard_capped() -> Result<(), ContextInspectionError> {
    let items = (0..(HARD_MAX_ITEMS + 1))
        .map(|index| message("user", &format!("item {index}")))
        .collect::<Vec<_>>();
    let limits = ContextInspectionLimits::with_max_items(HARD_MAX_ITEMS + 100);
    let snapshot =
        CurrentContextSnapshot::project(/*history_version*/ 0, &items, &items, limits)?;

    assert_eq!(snapshot.applied_limits.max_items(), HARD_MAX_ITEMS);
    assert_eq!(snapshot.raw.represented_items, to_u64(HARD_MAX_ITEMS));
    assert_eq!(snapshot.raw.omitted_items, 1);
    assert_eq!(snapshot.raw.items.len(), HARD_MAX_ITEMS);
    Ok(())
}

#[test]
fn zero_limit_returns_complete_aggregates_without_items() -> Result<(), ContextInspectionError> {
    let items = vec![message("user", "one"), message("assistant", "two")];
    let snapshot = CurrentContextSnapshot::project(
        /*history_version*/ 0,
        &items,
        &items,
        ContextInspectionLimits::with_max_items(/*max_items*/ 0),
    )?;

    assert_eq!(snapshot.raw.total_items, 2);
    assert_eq!(snapshot.raw.represented_items, 0);
    assert_eq!(snapshot.raw.omitted_items, 2);
    assert_eq!(snapshot.raw.items, Vec::new());
    assert!(snapshot.raw.total_serialized_bytes > 0);
    assert!(snapshot.raw.fingerprint.starts_with("sha256:"));
    Ok(())
}

#[test]
fn collection_fingerprint_is_deterministic_and_order_sensitive()
-> Result<(), ContextInspectionError> {
    let first = message("user", "first");
    let second = message("assistant", "second");
    let ordered = vec![first.clone(), second.clone()];
    let reversed = vec![second, first];

    let ordered_snapshot = CurrentContextSnapshot::project(
        /*history_version*/ 0,
        &ordered,
        &ordered,
        ContextInspectionLimits::default(),
    )?;
    let repeated_snapshot = CurrentContextSnapshot::project(
        /*history_version*/ 0,
        &ordered,
        &ordered,
        ContextInspectionLimits::default(),
    )?;
    let reversed_snapshot = CurrentContextSnapshot::project(
        /*history_version*/ 0,
        &reversed,
        &reversed,
        ContextInspectionLimits::default(),
    )?;

    assert_eq!(ordered_snapshot.raw, repeated_snapshot.raw);
    assert_ne!(
        ordered_snapshot.raw.fingerprint,
        reversed_snapshot.raw.fingerprint
    );
    Ok(())
}

#[test]
fn normalization_summary_compares_item_identities_as_a_multiset()
-> Result<(), ContextInspectionError> {
    let repeated = message("user", "same");
    let raw = vec![
        repeated.clone(),
        repeated.clone(),
        message("assistant", "raw only"),
    ];
    let normalized = vec![repeated, message("assistant", "normalized only")];

    let snapshot = CurrentContextSnapshot::project(
        /*history_version*/ 0,
        &raw,
        &normalized,
        ContextInspectionLimits::default(),
    )?;

    assert_eq!(
        snapshot.normalization,
        ContextNormalizationSummary {
            unchanged_items: 1,
            raw_only_items: 2,
            normalized_only_items: 1,
            changed: true,
        }
    );
    Ok(())
}

#[test]
fn normalization_summary_detects_order_only_changes() -> Result<(), ContextInspectionError> {
    let first = message("user", "first");
    let second = message("assistant", "second");
    let raw = vec![first.clone(), second.clone()];
    let normalized = vec![second, first];

    let snapshot = CurrentContextSnapshot::project(
        /*history_version*/ 0,
        &raw,
        &normalized,
        ContextInspectionLimits::default(),
    )?;

    assert_eq!(
        snapshot.normalization,
        ContextNormalizationSummary {
            unchanged_items: 2,
            raw_only_items: 0,
            normalized_only_items: 0,
            changed: true,
        }
    );
    Ok(())
}

#[test]
fn dynamic_metadata_is_utf8_bounded() -> Result<(), ContextInspectionError> {
    let long_role = "界".repeat(MAX_METADATA_TEXT_BYTES);
    let items = vec![message(&long_role, "text")];
    let snapshot = CurrentContextSnapshot::project(
        /*history_version*/ 0,
        &items,
        &items,
        ContextInspectionLimits::default(),
    )?;

    let role = snapshot.raw.items[0].role.as_deref().unwrap_or_default();
    assert!(role.len() <= MAX_METADATA_TEXT_BYTES);
    assert!(role.is_char_boundary(role.len()));
    Ok(())
}
