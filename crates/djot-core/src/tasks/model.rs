use std::ops::Range;
use std::path::PathBuf;

use crate::{RefTarget, TextEdit};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub range: Range<usize>,
    pub title_range: Option<Range<usize>>,
    pub title: String,
    pub depth: usize,
    pub id: Option<String>,
    pub created: Option<String>,
    pub done: Option<String>,
    pub canceled: Option<String>,
    pub due: Option<String>,
    pub wait: Option<String>,
    pub recur: Option<String>,
    pub prev: Option<String>,
    pub depends: Vec<TaskDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDependency {
    pub source: String,
    pub range: Range<usize>,
    pub target: RefTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStatusEdit {
    pub edits: Vec<TextEdit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Done,
    Canceled,
}

impl TaskStatus {
    pub(crate) fn attribute(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Canceled => "canceled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEditError {
    TaskIdNotFound { id: String },
    TaskAlreadyDone { id: String },
    TaskCanceled { id: String },
    CannotBuildEdit { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskRef {
    pub path: PathBuf,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTaskDependency {
    pub source: String,
    pub target: TaskRef,
    pub task: Task,
}
