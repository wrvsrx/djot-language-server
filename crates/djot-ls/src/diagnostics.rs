use std::path::Path;

use djot_core::{AnalysisDiagnostic, DiagnosticKind};
use lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString,
    PublishDiagnosticsParams, Url,
};

use crate::position::byte_range_to_lsp;
use crate::ServerState;

impl ServerState {
    pub(crate) fn publish_open_document_diagnostics(&self) {
        for path in &self.open_documents {
            self.publish_diagnostics_for(path);
        }
    }

    pub(crate) fn publish_diagnostics_for(&self, path: &Path) {
        let Some(entry) = self.workspace.get(path) else {
            return;
        };
        let Some(uri) = Url::from_file_path(path).ok() else {
            return;
        };
        let diagnostics = self
            .workspace
            .diagnostics_for(path)
            .into_iter()
            .map(|diagnostic| to_lsp_diagnostic(&entry.text, &uri, diagnostic))
            .collect();

        let _ = self
            .client
            .notify::<lsp_types::notification::PublishDiagnostics>(PublishDiagnosticsParams {
                uri,
                diagnostics,
                version: None,
            });
    }

    pub(crate) fn clear_diagnostics_for(&self, path: &Path) {
        let Some(uri) = Url::from_file_path(path).ok() else {
            return;
        };
        let _ = self
            .client
            .notify::<lsp_types::notification::PublishDiagnostics>(PublishDiagnosticsParams {
                uri,
                diagnostics: Vec::new(),
                version: None,
            });
    }
}

fn to_lsp_diagnostic(text: &str, uri: &Url, diagnostic: AnalysisDiagnostic) -> Diagnostic {
    let (code, message, related_information, severity) = match diagnostic.kind {
        DiagnosticKind::UnresolvedAnchor { id } => (
            "unresolved-anchor",
            format!("Unresolved anchor `{}`", id),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::UnresolvedPath { path } => (
            "unresolved-path",
            format!("Unresolved Djot path `{}`", path),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::DuplicateAnchor { id, first_range } => (
            "duplicate-anchor",
            format!("Duplicate anchor `{}`", id),
            Some(vec![DiagnosticRelatedInformation {
                location: Location::new(uri.clone(), byte_range_to_lsp(text, &first_range)),
                message: "First definition is here.".to_string(),
            }]),
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::MissingTaskDueForRecur => (
            "missing-task-due-for-recur",
            "Recurring tasks with `recur` need a valid RFC 3339 `due` datetime.".to_string(),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::InvalidTaskRecur { recur } => (
            "invalid-task-recur",
            format!(
                "Unsupported task `recur` value `{}`. Use an ISO 8601 duration like `P1D`, `P1W`, `P1M`, or `P1Y`.",
                recur
            ),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::ConflictingTaskClosedState => (
            "conflicting-task-closed-state",
            "Task cannot have both `done` and `canceled`.".to_string(),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::InvalidTaskPrevTarget { id } => (
            "invalid-task-prev-target",
            format!("Task `prev` target `{}` must be a task.", id),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::InvalidTaskDependencyTarget { target } => (
            "invalid-task-dependency-target",
            format!("Task dependency target `{}` must be a task.", target),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::TaskSelfDependency { target } => (
            "task-self-dependency",
            format!("Task cannot depend on itself via `{}`.", target),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::TaskDependencyCycle { id } => (
            "task-dependency-cycle",
            format!("Task dependency cycle includes `{}`.", id),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::TaskBlocked { count } => (
            "task-blocked",
            match count {
                1 => "Blocked by 1 open dependency.".to_string(),
                _ => format!("Blocked by {count} open dependencies."),
            },
            None,
            DiagnosticSeverity::HINT,
        ),
    };

    Diagnostic {
        range: byte_range_to_lsp(text, &diagnostic.range),
        severity: Some(severity),
        code: Some(NumberOrString::String(code.to_string())),
        code_description: None,
        source: Some("djot-ls".to_string()),
        message,
        related_information,
        tags: None,
        data: None,
    }
}
