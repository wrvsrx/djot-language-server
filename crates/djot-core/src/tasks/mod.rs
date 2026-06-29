mod attributes;
mod edits;
mod model;
mod recurrence;

#[cfg(test)]
pub(crate) use attributes::{anchor_attribute, filter_recurring_instance_attributes};
pub use edits::{task_list_item_conversion_edit, task_status_edits_at, task_status_edits_by_id};
pub use model::{
    ResolvedTaskDependency, Task, TaskDependency, TaskEditError, TaskRef, TaskStatus,
    TaskStatusEdit,
};
pub use recurrence::{next_recur_due, parse_repeat_rule, RepeatRule};

use crate::{AnalysisDiagnostic, DiagnosticKind};

pub(crate) fn document_local_task_diagnostics(tasks: &[Task]) -> Vec<AnalysisDiagnostic> {
    let mut diagnostics = Vec::new();

    for task in tasks {
        if task.done.is_some() && task.canceled.is_some() {
            diagnostics.push(AnalysisDiagnostic {
                range: task.range.clone(),
                kind: DiagnosticKind::ConflictingTaskClosedState,
            });
        }

        let Some(recur) = task.recur.as_deref() else {
            continue;
        };

        if parse_repeat_rule(recur).is_none() {
            diagnostics.push(AnalysisDiagnostic {
                range: task.range.clone(),
                kind: DiagnosticKind::InvalidTaskRecur {
                    recur: recur.to_string(),
                },
            });
        }

        if task.due.is_none() {
            diagnostics.push(AnalysisDiagnostic {
                range: task.range.clone(),
                kind: DiagnosticKind::MissingTaskDueForRecur,
            });
        }
    }

    diagnostics
}
