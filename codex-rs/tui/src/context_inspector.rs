use codex_app_server_protocol::ContextInspectionCollection;
use codex_app_server_protocol::ContextInspectionSnapshot;
use codex_app_server_protocol::ModelRequestAttemptStatus;
use codex_app_server_protocol::ModelRequestComponentSummary;
use codex_app_server_protocol::ModelRequestSnapshot;
use codex_app_server_protocol::ModelRequestTransport;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;

pub(crate) fn render_snapshot(snapshot: &ContextInspectionSnapshot) -> Vec<Line<'static>> {
    let mut lines = vec![
        "Live context snapshot".bold().into(),
        "Read-only, metadata-only. Content, arguments, outputs, schemas, and IDs are not disclosed."
            .dim()
            .into(),
        field("History version", snapshot.history_version.to_string()),
        field(
            "Item metadata limit",
            snapshot.applied_limits.max_items.to_string(),
        ),
    ];

    section(&mut lines, "Current history");
    collection(&mut lines, "Raw history", &snapshot.raw, "");
    if same_collection(&snapshot.raw, &snapshot.normalized) {
        lines.push(reference("Normalized history", "same as raw history"));
    } else {
        collection(&mut lines, "Normalized history", &snapshot.normalized, "");
    }
    lines.push(field(
        "Normalization changed",
        yes_no(snapshot.normalization.changed),
    ));
    lines.push(field(
        "Normalization identity counts",
        format!(
            "{} unchanged, {} raw-only, {} normalized-only",
            snapshot.normalization.unchanged_items,
            snapshot.normalization.raw_only_items,
            snapshot.normalization.normalized_only_items,
        ),
    ));

    section(&mut lines, "Observed model requests");
    lines.push(
        "Sent is transport-level: HTTP returned a response stream; WebSocket wrote the request frame."
            .dim()
            .into(),
    );
    match &snapshot.request.latest_attempt {
        Some(request) => request_snapshot(&mut lines, "Latest attempt", request),
        None => lines.push(reference("Latest attempt", "none observed")),
    }
    match &snapshot.request.last_actual {
        Some(actual)
            if snapshot
                .request
                .latest_attempt
                .as_ref()
                .is_some_and(|latest| latest.sequence == actual.sequence) =>
        {
            lines.push(reference(
                "Last actual",
                &format!("same as latest attempt (#{}), sent", actual.sequence),
            ));
        }
        Some(request) => request_snapshot(&mut lines, "Last actual", request),
        None => lines.push(reference("Last actual", "none sent")),
    }
    lines.push(field(
        "Current normalized matches last actual",
        optional_bool(snapshot.request.current_normalized_matches_last_actual),
    ));
    lines
}

pub(crate) fn render_error(error: &str) -> Vec<Line<'static>> {
    vec![
        "Context inspection failed".red().bold().into(),
        error.to_string().into(),
        "No model request was issued and no conversation history was changed."
            .dim()
            .into(),
    ]
}

fn section(lines: &mut Vec<Line<'static>>, title: &str) {
    lines.push("".into());
    lines.push(title.to_string().bold().underlined().into());
}

fn field(label: &str, value: String) -> Line<'static> {
    vec![format!("{label}: ").dim(), value.into()].into()
}

fn reference(label: &str, value: &str) -> Line<'static> {
    vec![format!("{label}: ").bold(), value.to_string().dim()].into()
}

fn collection(
    lines: &mut Vec<Line<'static>>,
    title: &str,
    collection: &ContextInspectionCollection,
    indent: &str,
) {
    lines.push(format!("{indent}{title}").bold().into());
    lines.push(
        format!(
            "{indent}  {} items · {} represented · {} omitted · {} serialized bytes",
            collection.total_items,
            collection.represented_items,
            collection.omitted_items,
            collection.total_serialized_bytes,
        )
        .into(),
    );
    lines.push(
        vec![
            format!("{indent}  Fingerprint: ").dim(),
            collection.fingerprint.clone().dim(),
        ]
        .into(),
    );
    if collection.items.is_empty() {
        lines.push(
            format!("{indent}  (no item metadata represented)")
                .dim()
                .into(),
        );
        return;
    }
    for item in &collection.items {
        let role = item
            .role
            .as_deref()
            .map(|role| format!(" · {role}"))
            .unwrap_or_default();
        lines.push(
            vec![
                format!("{indent}  [{:>3}] ", item.index).dim(),
                item.kind.clone().cyan(),
                role.into(),
                format!(" · {} B", item.serialized_bytes).dim(),
            ]
            .into(),
        );
    }
}

fn request_snapshot(lines: &mut Vec<Line<'static>>, title: &str, request: &ModelRequestSnapshot) {
    lines.push("".into());
    lines.push(
        vec![
            format!("{title} #{} · ", request.sequence).bold(),
            request_status(request.status),
        ]
        .into(),
    );
    lines.push(field(
        "  Captured at (Unix seconds)",
        request.captured_at.to_string(),
    ));
    lines.push(field(
        "  Model / provider",
        format!("{} / {}", request.model, request.provider),
    ));
    lines.push(field(
        "  Transport",
        format!(
            "{} · delta={} · connection reused={} · {} transport input items",
            transport(request.transport),
            yes_no(request.transport_uses_delta),
            yes_no(request.connection_reused),
            request.transport_input_items,
        ),
    ));
    lines.push(field(
        "  Logical request",
        format!("{} B · {}", request.serialized_bytes, request.fingerprint),
    ));
    component(lines, "  Instructions", request.instructions.as_ref());
    match &request.tools {
        Some(tools) => lines.push(field(
            "  Tools",
            format!(
                "{} items · {} B · {}",
                tools.total_items, tools.serialized_bytes, tools.fingerprint,
            ),
        )),
        None => lines.push(field("  Tools", "none".to_string())),
    }
    component(lines, "  Output schema", request.output_schema.as_ref());
    collection(
        lines,
        "Normalized request input",
        &request.normalized_input,
        "  ",
    );
    if same_collection(&request.normalized_input, &request.provider_input) {
        lines.push(
            vec![
                "  Provider request input: ".bold(),
                "same as normalized request input".dim(),
            ]
            .into(),
        );
    } else {
        collection(
            lines,
            "Provider request input",
            &request.provider_input,
            "  ",
        );
    }
}

fn component(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    component: Option<&ModelRequestComponentSummary>,
) {
    let value = component
        .map(|component| {
            format!(
                "{} B · {}",
                component.serialized_bytes, component.fingerprint
            )
        })
        .unwrap_or_else(|| "none".to_string());
    lines.push(field(label, value));
}

fn same_collection(
    left: &ContextInspectionCollection,
    right: &ContextInspectionCollection,
) -> bool {
    left.fingerprint == right.fingerprint && left.total_items == right.total_items
}

fn request_status(status: ModelRequestAttemptStatus) -> Span<'static> {
    match status {
        ModelRequestAttemptStatus::Prepared => "prepared".magenta(),
        ModelRequestAttemptStatus::Sent => "sent".green(),
        ModelRequestAttemptStatus::Failed => "failed".red(),
    }
}

fn transport(transport: ModelRequestTransport) -> &'static str {
    match transport {
        ModelRequestTransport::ResponsesHttp => "responses HTTP",
        ModelRequestTransport::ResponsesWebsocket => "responses WebSocket",
    }
}

fn yes_no(value: bool) -> String {
    if value {
        "yes".to_string()
    } else {
        "no".to_string()
    }
}

fn optional_bool(value: Option<bool>) -> String {
    value
        .map(yes_no)
        .unwrap_or_else(|| "not available".to_string())
}

#[cfg(test)]
#[path = "context_inspector_tests.rs"]
mod tests;
