use super::thread_processor::ThreadRequestProcessor;
use crate::error_code::internal_error;
use crate::error_code::invalid_request;
use codex_app_server_protocol::ClientResponsePayload;
use codex_app_server_protocol::ContextContentDisclosure;
use codex_app_server_protocol::ContextInspectionCollection;
use codex_app_server_protocol::ContextInspectionItem;
use codex_app_server_protocol::ContextInspectionLimits;
use codex_app_server_protocol::ContextInspectionSnapshot;
use codex_app_server_protocol::ContextNormalizationSummary;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ThreadContextInspectParams;
use codex_app_server_protocol::ThreadContextInspectResponse;
use codex_core::ContextInspectionLimits as CoreContextInspectionLimits;
use codex_core::ContextItemCollection as CoreContextItemCollection;
use codex_core::CurrentContextSnapshot;
use codex_protocol::ThreadId;

impl ThreadRequestProcessor {
    pub(crate) async fn thread_context_inspect(
        &self,
        params: ThreadContextInspectParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let thread_id = ThreadId::from_string(&params.thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;
        let thread = self
            .thread_manager
            .get_thread(thread_id)
            .await
            .map_err(|_| invalid_request(format!("thread not found: {thread_id}")))?;
        let limits = params
            .max_items
            .map(|max_items| {
                CoreContextInspectionLimits::with_max_items(
                    usize::try_from(max_items).unwrap_or(usize::MAX),
                )
            })
            .unwrap_or_default();
        let snapshot = thread
            .inspect_current_context(limits)
            .await
            .map_err(|err| internal_error(format!("failed to inspect thread context: {err}")))?;

        Ok(Some(
            ThreadContextInspectResponse {
                snapshot: api_context_inspection_snapshot(snapshot),
            }
            .into(),
        ))
    }
}

fn api_context_inspection_snapshot(snapshot: CurrentContextSnapshot) -> ContextInspectionSnapshot {
    ContextInspectionSnapshot {
        history_version: snapshot.history_version,
        content_disclosure: match snapshot.content_disclosure {
            codex_core::ContentDisclosure::MetadataOnly => ContextContentDisclosure::MetadataOnly,
        },
        applied_limits: ContextInspectionLimits {
            max_items: u64::try_from(snapshot.applied_limits.max_items()).unwrap_or(u64::MAX),
        },
        raw: api_context_inspection_collection(snapshot.raw),
        normalized: api_context_inspection_collection(snapshot.normalized),
        normalization: ContextNormalizationSummary {
            unchanged_items: snapshot.normalization.unchanged_items,
            raw_only_items: snapshot.normalization.raw_only_items,
            normalized_only_items: snapshot.normalization.normalized_only_items,
            changed: snapshot.normalization.changed,
        },
    }
}

fn api_context_inspection_collection(
    collection: CoreContextItemCollection,
) -> ContextInspectionCollection {
    ContextInspectionCollection {
        fingerprint: collection.fingerprint,
        total_items: collection.total_items,
        represented_items: collection.represented_items,
        omitted_items: collection.omitted_items,
        total_serialized_bytes: collection.total_serialized_bytes,
        items: collection
            .items
            .into_iter()
            .map(|item| ContextInspectionItem {
                index: item.index,
                kind: item.kind,
                role: item.role,
                serialized_bytes: item.serialized_bytes,
            })
            .collect(),
    }
}
