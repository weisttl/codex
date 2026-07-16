use super::*;
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
