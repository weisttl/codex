use std::sync::Arc;

use codex_context_inspector::ContextInspectionError;
use codex_context_inspector::ContextInspectionLimits;
use codex_context_inspector::CurrentContextSnapshot;
use codex_protocol::openai_models::InputModality;

use super::session::Session;
use crate::context_manager::ContextManager;

impl Session {
    pub(crate) async fn inspect_current_context(
        &self,
        limits: ContextInspectionLimits,
    ) -> Result<CurrentContextSnapshot, ContextInspectionError> {
        let (history, model, config) = {
            let state = self.state.lock().await;
            let session_configuration = &state.session_configuration;
            (
                state.clone_history(),
                session_configuration.collaboration_mode.model().to_string(),
                Arc::clone(&session_configuration.original_config_do_not_use),
            )
        };
        let model_info = self
            .services
            .models_manager
            .get_model_info(&model, &config.to_models_manager_config())
            .await;

        let request_inspection = self.services.model_client.inspect_model_requests(limits);
        inspect_history(&history, &model_info.input_modalities, limits)
            .map(|snapshot| snapshot.with_request_inspection(request_inspection))
    }
}

fn inspect_history(
    history: &ContextManager,
    input_modalities: &[InputModality],
    limits: ContextInspectionLimits,
) -> Result<CurrentContextSnapshot, ContextInspectionError> {
    let normalized = history.clone().for_prompt(input_modalities);
    CurrentContextSnapshot::project(
        history.history_version(),
        history.raw_items(),
        &normalized,
        limits,
    )
}

#[cfg(test)]
#[path = "context_inspection_tests.rs"]
mod tests;
