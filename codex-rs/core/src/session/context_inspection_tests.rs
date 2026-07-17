use super::*;
use codex_context_inspector::ContextNormalizationSummary;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ImageDetail;
use codex_protocol::models::ResponseItem;
use pretty_assertions::assert_eq;

use codex_utils_output_truncation::TruncationPolicy;

#[test]
fn inspection_uses_prompt_normalization_without_mutating_history()
-> Result<(), ContextInspectionError> {
    let item = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![
            ContentItem::InputText {
                text: "inspect this".to_string(),
            },
            ContentItem::InputImage {
                image_url: "https://example.com/image.png".to_string(),
                detail: Some(ImageDetail::Auto),
            },
        ],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    };
    let mut history = ContextManager::new();
    history.record_items(
        std::iter::once(&item),
        TruncationPolicy::Tokens(/*limit*/ 10_000),
    );
    let history_before = history.raw_items().to_vec();

    let first = inspect_history(
        &history,
        &[InputModality::Text],
        ContextInspectionLimits::default(),
    )?;
    let second = inspect_history(
        &history,
        &[InputModality::Text],
        ContextInspectionLimits::default(),
    )?;

    assert_eq!(first, second);
    assert_eq!(history.raw_items(), history_before);
    assert_eq!(first.raw.total_items, 1);
    assert_eq!(first.normalized.total_items, 1);
    assert_eq!(first.normalization.unchanged_items, 0);
    assert_eq!(first.normalization.raw_only_items, 1);
    assert_eq!(first.normalization.normalized_only_items, 1);
    assert!(first.normalization.changed);
    Ok(())
}

#[test]
fn inspection_of_empty_history_reports_empty_collections() -> Result<(), ContextInspectionError> {
    let history = ContextManager::new();

    let snapshot = inspect_history(
        &history,
        &[InputModality::Text],
        ContextInspectionLimits::default(),
    )?;

    assert_eq!(snapshot.history_version, history.history_version());
    assert_eq!(snapshot.raw.total_items, 0);
    assert_eq!(snapshot.normalized.total_items, 0);
    assert_eq!(
        snapshot.normalization,
        ContextNormalizationSummary {
            unchanged_items: 0,
            raw_only_items: 0,
            normalized_only_items: 0,
            changed: false,
        }
    );
    Ok(())
}

#[test]
fn inspection_reports_unchanged_normalization_for_plain_text_message()
-> Result<(), ContextInspectionError> {
    let item = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "no images here".to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    };
    let mut history = ContextManager::new();
    history.record_items(
        std::iter::once(&item),
        TruncationPolicy::Tokens(/*limit*/ 10_000),
    );

    let snapshot = inspect_history(
        &history,
        &[InputModality::Text],
        ContextInspectionLimits::default(),
    )?;

    assert_eq!(snapshot.raw.total_items, 1);
    assert_eq!(snapshot.normalized.total_items, 1);
    assert_eq!(snapshot.raw.fingerprint, snapshot.normalized.fingerprint);
    assert_eq!(
        snapshot.normalization,
        ContextNormalizationSummary {
            unchanged_items: 1,
            raw_only_items: 0,
            normalized_only_items: 0,
            changed: false,
        }
    );
    Ok(())
}

#[test]
fn inspection_reflects_synthetic_outputs_added_by_normalization()
-> Result<(), ContextInspectionError> {
    let call = ResponseItem::FunctionCall {
        id: None,
        name: "shell".to_string(),
        namespace: None,
        arguments: "{}".to_string(),
        call_id: "call-without-output".to_string(),
        internal_chat_message_metadata_passthrough: None,
    };
    let mut history = ContextManager::new();
    history.record_items(
        std::iter::once(&call),
        TruncationPolicy::Tokens(/*limit*/ 10_000),
    );

    let snapshot = inspect_history(
        &history,
        &[InputModality::Text],
        ContextInspectionLimits::default(),
    )?;

    // The lone function call has no matching output, so prompt normalization
    // synthesizes one; the raw history is untouched.
    assert_eq!(snapshot.raw.total_items, 1);
    assert_eq!(snapshot.normalized.total_items, 2);
    assert_eq!(
        snapshot.normalization,
        ContextNormalizationSummary {
            unchanged_items: 1,
            raw_only_items: 0,
            normalized_only_items: 1,
            changed: true,
        }
    );
    Ok(())
}

#[test]
fn inspection_applies_the_requested_limit_to_a_live_history()
-> Result<(), ContextInspectionError> {
    let items = (0..3)
        .map(|index| ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: format!("item {index}"),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        })
        .collect::<Vec<_>>();
    let mut history = ContextManager::new();
    history.record_items(items.iter(), TruncationPolicy::Tokens(/*limit*/ 10_000));

    let snapshot = inspect_history(
        &history,
        &[InputModality::Text],
        ContextInspectionLimits::with_max_items(/*max_items*/ 0),
    )?;

    assert_eq!(snapshot.raw.total_items, 3);
    assert_eq!(snapshot.raw.represented_items, 0);
    assert_eq!(snapshot.raw.omitted_items, 3);
    assert_eq!(snapshot.raw.items, Vec::new());
    Ok(())
}
