use codex_core::ContextInspectionLimits;
use codex_core::CurrentContextSnapshot;
use codex_protocol::models::ResponseItem;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inspection_is_repeatable_and_matches_the_next_request_prefix() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let first_request = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("response-1"),
            ev_completed("response-1"),
        ]),
    )
    .await;
    let second_request = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("response-2"),
            ev_completed("response-2"),
        ]),
    )
    .await;
    let test = test_codex().build_with_auto_env(&server).await?;

    test.submit_turn("first turn").await?;
    let _ = first_request.single_request();

    let inspected = test
        .codex
        .inspect_current_context(ContextInspectionLimits::default())
        .await?;
    let repeated = test
        .codex
        .inspect_current_context(ContextInspectionLimits::default())
        .await?;
    assert_eq!(inspected, repeated);

    test.submit_turn("second turn").await?;
    let request_input = second_request
        .single_request()
        .input()
        .into_iter()
        .map(serde_json::from_value)
        .collect::<serde_json::Result<Vec<ResponseItem>>>()?;
    let inspected_items = usize::try_from(inspected.normalized.total_items)?;
    assert_eq!(request_input.len(), inspected_items + 1);

    let request_prefix = &request_input[..inspected_items];
    let prefix_projection = CurrentContextSnapshot::project(
        /*history_version*/ 0,
        request_prefix,
        request_prefix,
        ContextInspectionLimits::with_max_items(/*max_items*/ 0),
    )?;
    assert_eq!(
        inspected.normalized.fingerprint,
        prefix_projection.normalized.fingerprint
    );

    Ok(())
}
