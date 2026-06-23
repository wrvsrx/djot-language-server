//! Protocol-agnostic djot document analysis shared by the language server and
//! (in the future) the exporter.
//!
//! Everything here works in **byte offsets** into the source text. Consumers
//! that need editor coordinates (LSP UTF-16 positions) or a particular AST
//! (pandoc) convert at their own boundary - this crate never depends on those.

mod analysis;
mod diagnostics;
mod edits;
mod paths;
mod references;
mod rename;
mod tasks;
mod workspace;

pub use analysis::{
    analyze, build_index, has_class, heading_outline, metadata_block, metadata_insertion_edit,
    tasks, Analysis, Anchor, DocIndex, Heading,
};
pub use diagnostics::{AnalysisDiagnostic, DiagnosticKind};
pub use edits::{
    apply_text_edits, DocumentTextEdit, EditError, FileRenameEdit, TextEdit, WorkspaceEdit,
};
pub use references::{
    parse_dst, resolve_target, RefTarget, Reference, ReferenceKind, ResolvedTarget,
};
pub use rename::{PathRenameError, PathRenameTarget, RenameTarget, RenameTargetError};
#[cfg(test)]
pub(crate) use tasks::{anchor_attribute, filter_recurring_instance_attributes};
pub use tasks::{
    next_recur_due, parse_repeat_rule, task_done_edits_by_id, task_list_item_conversion_edit,
    task_status_edits_at, RepeatRule, ResolvedTaskDependency, Task, TaskDependency, TaskEditError,
    TaskRef, TaskStatus, TaskStatusEdit,
};
pub use workspace::{DocEntry, Workspace};

/// The class that marks a leading code block as document metadata. This is a
/// djot-ls / djot-export convention layered on djot's native attribute syntax,
/// not part of djot itself - other djot tools simply see a classed code block.
pub const METADATA_CLASS: &str = "metadata";
pub const TASK_CLASS: &str = "task";

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use jotdown::{Container, Event, Parser};
    use std::path::{Path, PathBuf};

    struct WorkspaceFixture {
        workspace: Workspace,
        index: PathBuf,
        topic: PathBuf,
        renamed: PathBuf,
        index_text: &'static str,
    }

    fn workspace_fixture() -> WorkspaceFixture {
        let index = PathBuf::from("/notes/index.dj");
        let topic = PathBuf::from("/notes/topic.dj");
        let renamed = PathBuf::from("/notes/sub/renamed.dj");
        let index_text = "# Index\n\n[topic](topic.dj#topic) [missing](missing.dj)\n\n{#blocked created=\"2026-06-18T09:00:00Z\" depends=\"#open\"}\n::: task\nBlocked task.\n:::\n\n{#open created=\"2026-06-18T09:00:00Z\"}\n::: task\nOpen task.\n:::\n";
        let topic_text = "{#topic}\nTopic\n";
        let mut workspace = Workspace::new();
        workspace.insert(index.clone(), index_text.to_string());
        workspace.insert(topic.clone(), topic_text.to_string());
        WorkspaceFixture {
            workspace,
            index,
            topic,
            renamed,
            index_text,
        }
    }

    #[test]
    fn outline_nests_by_section_level() {
        let text = "# A\n\ntext\n\n## B\n\n### C\n\n# D\n";
        let roots = heading_outline(text);
        assert_eq!(
            roots.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(),
            ["A", "D"]
        );
        let a = &roots[0];
        assert_eq!(a.level, 1);
        assert_eq!(
            a.children
                .iter()
                .map(|h| h.name.as_str())
                .collect::<Vec<_>>(),
            ["B"]
        );
        assert_eq!(a.children[0].children[0].name, "C");
        // Parent section range encloses its children.
        assert!(a.range.end >= a.children[0].range.end);
    }

    #[test]
    fn index_collects_anchors_and_references() {
        let text = "# My Heading\n\n[a](#My-Heading) [b][] [u](https://x.y) [f](o.dj#s)\n\n## b\n";
        let index = build_index(text);
        assert!(index.anchors.contains_key("My-Heading"));
        assert!(index.anchors.contains_key("b"));

        let targets: Vec<_> = index.references.iter().map(|r| &r.target).collect();
        assert!(targets.contains(&&RefTarget::Internal {
            id: "My-Heading".into()
        }));
        assert!(targets.contains(&&RefTarget::Url("https://x.y".into())));
        assert!(targets.contains(&&RefTarget::External {
            path: "o.dj".into(),
            id: Some("s".into()),
        }));
    }

    #[test]
    fn analysis_collects_shared_document_semantics() {
        let text = "{.metadata}\n``` toml\ntitle = \"x\"\n```\n\n# Topic\n\n{#task-a recur=\"P1Q\"}\n::: task\nTask A.\n:::\n\n[topic](#Topic)\n";
        let analysis = analyze(text);

        assert_eq!(analysis.metadata.as_deref(), Some("title = \"x\"\n"));
        assert!(analysis.index.anchors.contains_key("Topic"));
        assert_eq!(analysis.index.references.len(), 1);
        assert_eq!(analysis.tasks.len(), 1);
        assert_eq!(analysis.tasks[0].id.as_deref(), Some("task-a"));
        assert!(analysis.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::InvalidTaskRecur {
                    recur: "P1Q".into(),
                }
        }));
        assert!(analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::MissingTaskDueForRecur));
    }

    #[test]
    fn index_tracks_anchor_rename_ranges() {
        let text = "# My Heading\n\n{#custom}\nparagraph\n\n{prev=\"#quoted\" id=\"quoted\"}\nquoted paragraph\n\n{id=bare}\nbare paragraph\n\n{id=\"学习-anki\"}\nunicode paragraph\n";
        let index = build_index(text);

        let heading = &index.anchors["My-Heading"];
        assert_eq!(&text[heading.rename_range.clone()], "My Heading");
        assert!(!heading.explicit);

        let explicit = &index.anchors["custom"];
        assert_eq!(&text[explicit.rename_range.clone()], "custom");
        assert!(explicit.explicit);

        let quoted = &index.anchors["quoted"];
        assert_eq!(&text[quoted.rename_range.clone()], "quoted");
        assert!(quoted.explicit);

        let bare = &index.anchors["bare"];
        assert_eq!(&text[bare.rename_range.clone()], "bare");
        assert!(bare.explicit);

        let unicode = &index.anchors["学习-anki"];
        assert_eq!(&text[unicode.rename_range.clone()], "学习-anki");
        assert!(unicode.explicit);
    }

    #[test]
    fn index_tracks_reference_target_id_ranges() {
        let text = "[internal](#Topic) [external](other.dj#Section) [file](other.dj) [implicit][]";
        let index = build_index(text);

        let ranges = index
            .references
            .iter()
            .filter_map(|reference| {
                reference
                    .target_id_range
                    .clone()
                    .map(|range| text[range].to_string())
            })
            .collect::<Vec<_>>();

        assert_eq!(ranges, ["Topic", "Section"]);
    }

    #[test]
    fn index_tracks_reference_target_path_ranges() {
        let text = "[internal](#Topic) [external](other.dj#Section) [file](notes/other.dj) [url](https://example.com)";
        let index = build_index(text);

        let ranges = index
            .references
            .iter()
            .filter_map(|reference| {
                reference
                    .target_path_range
                    .clone()
                    .map(|range| text[range].to_string())
            })
            .collect::<Vec<_>>();

        assert_eq!(ranges, ["other.dj", "notes/other.dj"]);
    }

    #[test]
    fn index_tracks_task_prev_references() {
        let text = "{prev=\"#old-task\"}\n::: task\nNext task.\n:::\n\n{prev=\"other.dj#previous\"}\n::: task\nCross-file next task.\n:::\n\n{prev=\"other.dj\"}\n::: task\nFile-only prev is not a reference.\n:::\n";
        let index = build_index(text);

        let refs = index
            .references
            .iter()
            .map(|reference| {
                (
                    text[reference.source.clone()].to_string(),
                    reference
                        .target_path_range
                        .clone()
                        .map(|range| text[range].to_string()),
                    reference
                        .target_id_range
                        .clone()
                        .map(|range| text[range].to_string()),
                    reference.target.clone(),
                    reference.kind,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            refs,
            vec![
                (
                    "#old-task".to_string(),
                    None,
                    Some("old-task".to_string()),
                    RefTarget::Internal {
                        id: "old-task".to_string()
                    },
                    ReferenceKind::TaskPrev,
                ),
                (
                    "other.dj#previous".to_string(),
                    Some("other.dj".to_string()),
                    Some("previous".to_string()),
                    RefTarget::External {
                        path: "other.dj".to_string(),
                        id: Some("previous".to_string()),
                    },
                    ReferenceKind::TaskPrev,
                ),
            ]
        );
    }

    #[test]
    fn metadata_block_extracts_leading_toml() {
        let text = "{.metadata}\n``` toml\ntitle = \"x\"\n```\n\n# H\n";
        assert_eq!(metadata_block(text).as_deref(), Some("title = \"x\"\n"));
        // A plain code block is not metadata.
        assert_eq!(metadata_block("``` toml\ntitle = \"x\"\n```\n"), None);
    }

    #[test]
    fn tasks_extract_task_divs() {
        let text = "{#write-parser}\n{created=\"2026-06-18T09:00:00+08:00\" due=\"2026-06-20T09:00:00+08:00\" wait=\"2026-06-19T09:00:00+08:00\" done=\"2026-06-19T21:30:00+08:00\" canceled=\"2026-06-19T22:00:00+08:00\" recur=\"P1W\" prev=\"#previous-task\"}\n::: task\nWrite parser.\n\nDetails.\n:::\n\n::: note\nNot a task.\n:::\n";
        let found = tasks(text);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id.as_deref(), Some("write-parser"));
        assert_eq!(
            found[0].created.as_deref(),
            Some("2026-06-18T09:00:00+08:00")
        );
        assert_eq!(found[0].done.as_deref(), Some("2026-06-19T21:30:00+08:00"));
        assert_eq!(
            found[0].canceled.as_deref(),
            Some("2026-06-19T22:00:00+08:00")
        );
        assert_eq!(found[0].due.as_deref(), Some("2026-06-20T09:00:00+08:00"));
        assert_eq!(found[0].wait.as_deref(), Some("2026-06-19T09:00:00+08:00"));
        assert_eq!(found[0].recur.as_deref(), Some("P1W"));
        assert_eq!(found[0].prev.as_deref(), Some("#previous-task"));
        assert_eq!(found[0].title, "Write parser.");
        assert_eq!(
            found[0]
                .title_range
                .clone()
                .map(|range| text[range].to_string()),
            Some("Write parser.".to_string())
        );
    }

    #[test]
    fn tasks_inherit_metadata_from_containing_list_item() {
        let text = "- {#write-parser created=\"2026-06-18T09:00:00Z\" canceled=\"2026-06-18T18:00:00Z\" due=\"2026-06-19T09:00:00Z\" wait=\"2026-06-18T21:00:00Z\" recur=\"P1D\" prev=\"#previous-task\"}\n  ::: task\n  Write parser.\n  :::\n";
        let found = tasks(text);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id.as_deref(), Some("write-parser"));
        assert_eq!(found[0].created.as_deref(), Some("2026-06-18T09:00:00Z"));
        assert_eq!(found[0].due.as_deref(), Some("2026-06-19T09:00:00Z"));
        assert_eq!(found[0].wait.as_deref(), Some("2026-06-18T21:00:00Z"));
        assert_eq!(found[0].recur.as_deref(), Some("P1D"));
        assert_eq!(found[0].prev.as_deref(), Some("#previous-task"));
        assert_eq!(found[0].done, None);
        assert_eq!(found[0].canceled.as_deref(), Some("2026-06-18T18:00:00Z"));
        assert_eq!(found[0].title, "Write parser.");
    }

    #[test]
    fn tasks_report_depth_for_nested_task_divs() {
        let text = "::: task\nParent.\n\n::: task\nChild.\n\n::: task\nGrandchild.\n:::\n:::\n:::\n\n::: task\nSibling.\n:::\n";
        let found = tasks(text);

        assert_eq!(found.len(), 4);
        assert_eq!(
            found
                .iter()
                .map(|task| (task.title.as_str(), task.depth))
                .collect::<Vec<_>>(),
            vec![
                ("Parent.", 0),
                ("Child.", 1),
                ("Grandchild.", 2),
                ("Sibling.", 0)
            ]
        );
    }

    #[test]
    fn tasks_extract_dependency_tokens() {
        let text =
            "{depends=\"#draft #review other%20file.dj#publish\"}\n::: task\nBlocked task.\n:::\n";
        let found = tasks(text);

        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0]
                .depends
                .iter()
                .map(|dependency| (dependency.source.as_str(), &dependency.target))
                .collect::<Vec<_>>(),
            vec![
                (
                    "#draft",
                    &RefTarget::Internal {
                        id: "draft".to_string()
                    }
                ),
                (
                    "#review",
                    &RefTarget::Internal {
                        id: "review".to_string()
                    }
                ),
                (
                    "other%20file.dj#publish",
                    &RefTarget::External {
                        path: "other file.dj".to_string(),
                        id: Some("publish".to_string())
                    }
                ),
            ]
        );
        assert_eq!(
            found[0]
                .depends
                .iter()
                .map(|dependency| text[dependency.range.clone()].to_string())
                .collect::<Vec<_>>(),
            vec!["#draft", "#review", "other%20file.dj#publish"]
        );
    }

    #[test]
    fn tasks_prefer_div_wait_over_containing_list_item() {
        let text = "- {wait=\"2026-06-18T21:00:00Z\"}\n  {wait=\"2026-06-19T09:00:00Z\"}\n  ::: task\n  Write parser.\n  :::\n";
        let found = tasks(text);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].wait.as_deref(), Some("2026-06-19T09:00:00Z"));
    }

    #[test]
    fn tasks_reject_date_only_datetime_attributes() {
        let text = "{created=\"2026-06-18\" done=2026-06-19 canceled=2026-06-20 wait=\"2026-06-21\"}\n::: task\nDate-only metadata.\n:::\n\n{created=\"2026-06-18T09:00:00Z\" done=\"2026-06-19T13:30:00Z\" canceled=\"2026-06-20T13:30:00Z\" wait=\"2026-06-21T09:00:00Z\"}\n::: task\nDatetime metadata.\n:::\n";
        let found = tasks(text);

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].created, None);
        assert_eq!(found[0].done, None);
        assert_eq!(found[0].canceled, None);
        assert_eq!(found[0].wait, None);
        assert_eq!(found[1].created.as_deref(), Some("2026-06-18T09:00:00Z"));
        assert_eq!(found[1].done.as_deref(), Some("2026-06-19T13:30:00Z"));
        assert_eq!(found[1].canceled.as_deref(), Some("2026-06-20T13:30:00Z"));
        assert_eq!(found[1].wait.as_deref(), Some("2026-06-21T09:00:00Z"));
    }

    #[test]
    fn task_done_edits_by_id_mark_task_done() {
        let text = "{#write-parser}\n::: task\nWrite parser.\n:::\n";
        let edits =
            task_done_edits_by_id(text, "write-parser", "2026-06-22T09:00:00+08:00").unwrap();
        let updated = apply_text_edits(text.to_string(), edits).unwrap();

        assert_eq!(
            updated,
            "{#write-parser}\n{done=\"2026-06-22T09:00:00+08:00\"}\n::: task\nWrite parser.\n:::\n"
        );
    }

    #[test]
    fn metadata_insertion_edit_adds_leading_metadata_block() {
        let text = "\n\n# Heading\n";
        let edit = metadata_insertion_edit(
            text,
            1,
            Path::new("/notes/my \"note\".dj"),
            "2026-06-22T09:00:00+08:00",
        )
        .unwrap();

        assert_eq!(edit.range, 0..0);
        assert_eq!(
            edit.new_text,
            "{.metadata}\n``` toml\ntitle = \"my \\\"note\\\"\"\ncreated = \"2026-06-22T09:00:00+08:00\"\n```\n\n"
        );
        assert!(metadata_insertion_edit("# Heading\n", 2, Path::new("x.dj"), "now").is_none());
    }

    #[test]
    fn task_list_item_conversion_edit_converts_open_native_task() {
        let text = "# Tasks\n\n  - [ ] Write parser.\n";
        let edit =
            task_list_item_conversion_edit(text, text.find("Write").unwrap(), "created").unwrap();

        assert_eq!(&text[edit.range.clone()], "  - [ ] Write parser.");
        assert_eq!(
            edit.new_text,
            "  - {created=\"created\"}\n    ::: task\n    Write parser.\n    :::"
        );
    }

    #[test]
    fn resolve_target_handles_internal_relative_and_url() {
        let from = PathBuf::from("/notes/sub/a.dj");
        assert_eq!(
            resolve_target(&from, &RefTarget::Internal { id: "x".into() }).unwrap(),
            ResolvedTarget {
                path: from.clone(),
                id: Some("x".into())
            }
        );
        assert_eq!(
            resolve_target(
                &from,
                &RefTarget::External {
                    path: "../b.dj".into(),
                    id: Some("y".into())
                }
            )
            .unwrap(),
            ResolvedTarget {
                path: PathBuf::from("/notes/b.dj"),
                id: Some("y".into())
            }
        );
        assert!(resolve_target(&from, &RefTarget::Url("https://x".into())).is_none());
    }

    #[test]
    fn workspace_cross_file_definition_and_backref() {
        let a = PathBuf::from("/notes/a.dj");
        let b = PathBuf::from("/notes/b.dj");
        let doc_a = "# A\n\nsee [to B](b.dj#Topic)\n";
        let mut ws = Workspace::new();
        ws.insert(a.clone(), doc_a.to_string());
        ws.insert(b.clone(), "# Topic\n\ntext\n".to_string());

        // Cursor on the link in a.dj resolves to b.dj#Topic, which exists.
        let offset = doc_a.find("b.dj").unwrap();
        let reference = ws.reference_at(&a, offset).expect("reference under cursor");
        let resolved = resolve_target(&a, &reference.target).expect("resolved");
        assert_eq!(resolved.path, b);
        assert_eq!(resolved.id.as_deref(), Some("Topic"));
        assert!(ws.anchor(&resolved.path, "Topic").is_some());
        let topic_text_offset = ws.get(&b).unwrap().text.find("Topic").unwrap();
        assert_eq!(ws.anchor_at(&b, topic_text_offset).unwrap().0, "Topic");

        // Backward: exactly one document references (b.dj, Topic).
        let back = ws.references_to(&b, "Topic");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].0, a);
    }

    #[test]
    fn workspace_resolves_rename_target_from_anchor_or_reference() {
        let a = PathBuf::from("/notes/a.dj");
        let b = PathBuf::from("/notes/b.dj");
        let doc_a = "# A\n\nsee [to B](b.dj#topic)\n";
        let doc_b = "{#topic}\nTopic\n";
        let mut ws = Workspace::new();
        ws.insert(a.clone(), doc_a.to_string());
        ws.insert(b.clone(), doc_b.to_string());

        let from_anchor = ws
            .rename_target_at(&b, doc_b.find("topic").unwrap())
            .expect("rename target from anchor");
        assert_eq!(from_anchor.path, b);
        assert_eq!(from_anchor.id, "topic");
        assert_eq!(&doc_b[from_anchor.range], "topic");

        let from_reference = ws
            .rename_target_at(&a, doc_a.find("topic").unwrap())
            .expect("rename target from reference");
        assert_eq!(from_reference.path, PathBuf::from("/notes/b.dj"));
        assert_eq!(from_reference.id, "topic");
        assert_eq!(&doc_a[from_reference.range], "topic");
        assert_eq!(
            ws.rename_target_at(&a, doc_a.find("b.dj").unwrap()),
            Err(RenameTargetError::NotRenameable)
        );
    }

    #[test]
    fn workspace_renames_anchor_only_from_rename_range() {
        let path = PathBuf::from("/notes/tasks.dj");
        let doc = "{#topic}\n::: task\nTask title.\n:::\n\n- {#list-task}\n  ::: task\n  List task title.\n  :::\n";
        let mut ws = Workspace::new();
        ws.insert(path.clone(), doc.to_string());

        let from_anchor = ws
            .rename_target_at(&path, doc.find("topic").unwrap())
            .expect("rename target from explicit anchor");
        assert_eq!(from_anchor.id, "topic");
        assert_eq!(&doc[from_anchor.range], "topic");

        let from_list_anchor = ws
            .rename_target_at(&path, doc.find("list-task").unwrap())
            .expect("rename target from list item anchor");
        assert_eq!(from_list_anchor.id, "list-task");
        assert_eq!(&doc[from_list_anchor.range], "list-task");

        assert_eq!(
            ws.rename_target_at(&path, doc.find("Task title").unwrap()),
            Err(RenameTargetError::NotRenameable)
        );
        assert_eq!(
            ws.rename_target_at(&path, doc.find("List task title").unwrap()),
            Err(RenameTargetError::NotRenameable)
        );
    }

    #[test]
    fn workspace_collects_anchor_rename_edits() {
        let a = PathBuf::from("/notes/a.dj");
        let b = PathBuf::from("/notes/b.dj");
        let doc_a =
            "# A\n\n[local](#A) [other](b.dj#topic) [file](b.dj)\n\n{prev=\"b.dj#topic\"}\n::: task\nNext.\n:::\n";
        let doc_b = "{#topic}\nTopic\n\n[back](../notes/a.dj#A)\n";
        let mut ws = Workspace::new();
        ws.insert(a.clone(), doc_a.to_string());
        ws.insert(b.clone(), doc_b.to_string());

        let mut document_edits = ws
            .anchor_rename_edits(&b, "topic", "renamed")
            .into_iter()
            .map(|edit| {
                let text = &ws.get(&edit.path).unwrap().text;
                (
                    edit.path,
                    text[edit.edit.range].to_string(),
                    edit.edit.new_text,
                )
            })
            .collect::<Vec<_>>();
        document_edits.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(
            document_edits,
            vec![
                (
                    PathBuf::from("/notes/a.dj"),
                    "topic".to_string(),
                    "renamed".to_string()
                ),
                (
                    PathBuf::from("/notes/a.dj"),
                    "topic".to_string(),
                    "renamed".to_string()
                ),
                (
                    PathBuf::from("/notes/b.dj"),
                    "topic".to_string(),
                    "renamed".to_string()
                )
            ]
        );
    }

    #[test]
    fn workspace_rejects_rename_for_implicit_heading_anchor() {
        let a = PathBuf::from("/notes/a.dj");
        let b = PathBuf::from("/notes/b.dj");
        let doc_a = "# A\n\nsee [to B](b.dj#Topic)\n";
        let doc_b = "# Topic\n";
        let mut ws = Workspace::new();
        ws.insert(a.clone(), doc_a.to_string());
        ws.insert(b.clone(), doc_b.to_string());

        assert_eq!(
            ws.rename_target_at(&b, doc_b.find("Topic").unwrap()),
            Err(RenameTargetError::ImplicitHeadingAnchor)
        );
        assert_eq!(
            ws.rename_target_at(&a, doc_a.find("Topic").unwrap()),
            Err(RenameTargetError::ImplicitHeadingAnchor)
        );
        assert!(ws.anchor_rename_edits(&b, "Topic", "Renamed").is_empty());
    }

    #[test]
    fn workspace_resolves_path_rename_target_from_link_path() {
        let a = PathBuf::from("/notes/a.dj");
        let b = PathBuf::from("/notes/b.dj");
        let doc_a = "# A\n\nsee [to B](b.dj#topic)\n";
        let mut ws = Workspace::new();
        ws.insert(a.clone(), doc_a.to_string());
        ws.insert(b.clone(), "{#topic}\nTopic\n".to_string());

        let target = ws
            .path_rename_target_at(&a, doc_a.find("b.dj").unwrap())
            .expect("path rename target");

        assert_eq!(target.old_path, b);
        assert_eq!(&doc_a[target.range], "b.dj");
        assert_eq!(
            ws.path_rename_target_at(&a, doc_a.find("topic").unwrap()),
            Err(PathRenameError::NotRenameable)
        );
    }

    #[test]
    fn workspace_collects_path_rename_edit_plan_with_relative_replacements() {
        let a = PathBuf::from("/notes/a.dj");
        let b = PathBuf::from("/notes/b.dj");
        let c = PathBuf::from("/notes/sub/c.dj");
        let renamed = PathBuf::from("/notes/renamed.dj");
        let doc_a = "# A\n\n[topic](b.dj#topic)\n";
        let doc_c = "# C\n\n[topic](../b.dj)\n";
        let mut ws = Workspace::new();
        ws.insert(a.clone(), doc_a.to_string());
        ws.insert(b.clone(), "{#topic}\nTopic\n".to_string());
        ws.insert(c.clone(), doc_c.to_string());

        let plan = ws.path_rename_edit_plan(&b, &renamed);
        assert_eq!(
            plan.first(),
            Some(&WorkspaceEdit::RenameFile(FileRenameEdit {
                old_path: b,
                new_path: renamed,
            }))
        );

        let mut text_edits = plan
            .into_iter()
            .filter_map(|edit| match edit {
                WorkspaceEdit::Text(edit) => {
                    let text = &ws.get(&edit.path).unwrap().text;
                    Some((
                        edit.path,
                        text[edit.edit.range].to_string(),
                        edit.edit.new_text,
                    ))
                }
                WorkspaceEdit::RenameFile(_) => None,
            })
            .collect::<Vec<_>>();
        text_edits.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(
            text_edits,
            vec![
                (
                    PathBuf::from("/notes/a.dj"),
                    "b.dj".to_string(),
                    "renamed.dj".to_string()
                ),
                (
                    PathBuf::from("/notes/sub/c.dj"),
                    "../b.dj".to_string(),
                    "../renamed.dj".to_string()
                ),
            ]
        );
    }

    #[test]
    fn workspace_fixture_covers_diagnostics_and_edit_plans() {
        let fixture = workspace_fixture();
        let ws = fixture.workspace;

        let diagnostics = ws.diagnostics_for(&fixture.index);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::UnresolvedPath {
                    path: "missing.dj".to_string(),
                }
        }));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.kind == DiagnosticKind::TaskBlocked { count: 1 } }));

        let mut anchor_edits = ws
            .anchor_rename_edits(&fixture.topic, "topic", "renamed")
            .into_iter()
            .map(|edit| {
                let text = &ws.get(&edit.path).unwrap().text;
                (
                    edit.path,
                    text[edit.edit.range].to_string(),
                    edit.edit.new_text,
                )
            })
            .collect::<Vec<_>>();
        anchor_edits.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            anchor_edits,
            vec![
                (
                    fixture.index.clone(),
                    "topic".to_string(),
                    "renamed".to_string()
                ),
                (
                    fixture.topic.clone(),
                    "topic".to_string(),
                    "renamed".to_string()
                ),
            ]
        );

        let plan = ws.path_rename_edit_plan(&fixture.topic, &fixture.renamed);
        assert_eq!(
            plan.first(),
            Some(&WorkspaceEdit::RenameFile(FileRenameEdit {
                old_path: fixture.topic.clone(),
                new_path: fixture.renamed,
            }))
        );
        assert!(plan.iter().any(|edit| match edit {
            WorkspaceEdit::Text(edit) => {
                let text = &ws.get(&edit.path).unwrap().text;
                edit.path == fixture.index
                    && &text[edit.edit.range.clone()] == "topic.dj"
                    && edit.edit.new_text == "sub/renamed.dj"
            }
            WorkspaceEdit::RenameFile(_) => false,
        }));

        let edits =
            task_done_edits_by_id(fixture.index_text, "open", "2026-06-19T09:00:00Z").unwrap();
        let updated = apply_text_edits(fixture.index_text.to_string(), edits).unwrap();
        assert!(updated.contains("{done=\"2026-06-19T09:00:00Z\"}"));
    }

    #[test]
    fn larger_workspace_fixture_covers_paths_duplicates_cycles_and_rename_edits() {
        let root = PathBuf::from("/notes");
        let index = root.join("index.dj");
        let project = root.join("Project Plan.dj");
        let nested = root.join("nested/Work File.dj");
        let renamed = root.join("archive/Project Plan.dj");
        let index_text = "# Index\n\n[review](Project Plan.dj#review) [topic](nested/Work File.dj#topic)\n\n{#publish depends=\"Project%20Plan.dj#review\"}\n::: task\nPublish.\n:::\n";
        let project_text = "{#review depends=\"nested/Work%20File.dj#draft\"}\n::: task\nReview.\n:::\n\n{id=\"review\"}\nDuplicate review anchor.\n";
        let nested_text =
            "{#topic}\n# Topic\n\n{#draft depends=\"../Project%20Plan.dj#review\"}\n::: task\nDraft.\n:::\n";
        let mut ws = Workspace::new();
        ws.insert(index.clone(), index_text.to_string());
        ws.insert(project.clone(), project_text.to_string());
        ws.insert(nested.clone(), nested_text.to_string());

        let index_diagnostics = ws.diagnostics_for(&index);
        assert!(index_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::TaskBlocked { count: 1 }));

        let project_diagnostics = ws.diagnostics_for(&project);
        assert!(project_diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::DuplicateAnchor {
                    id: "review".to_string(),
                    first_range: 2..8,
                }
        }));
        assert!(project_diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::TaskDependencyCycle {
                    id: "review".to_string(),
                }
        }));

        let nested_diagnostics = ws.diagnostics_for(&nested);
        assert!(nested_diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::TaskDependencyCycle {
                    id: "draft".to_string(),
                }
        }));

        let publish = ws.task_by_id(&index, "publish").unwrap();
        assert_eq!(
            ws.open_task_dependencies(&index, &publish)
                .into_iter()
                .map(|dependency| dependency.target)
                .collect::<Vec<_>>(),
            vec![TaskRef {
                path: project.clone(),
                id: "review".to_string(),
            }]
        );

        let mut anchor_edits = ws
            .anchor_rename_edits(&project, "review", "review-done")
            .into_iter()
            .map(|edit| {
                let text = &ws.get(&edit.path).unwrap().text;
                (
                    edit.path,
                    text[edit.edit.range].to_string(),
                    edit.edit.new_text,
                )
            })
            .collect::<Vec<_>>();
        anchor_edits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        assert_eq!(
            anchor_edits,
            vec![
                (
                    project.clone(),
                    "review".to_string(),
                    "review-done".to_string()
                ),
                (
                    index.clone(),
                    "review".to_string(),
                    "review-done".to_string()
                ),
                (
                    index.clone(),
                    "review".to_string(),
                    "review-done".to_string()
                ),
                (
                    nested.clone(),
                    "review".to_string(),
                    "review-done".to_string()
                ),
            ]
        );

        let plan = ws.path_rename_edit_plan(&project, &renamed);
        assert_eq!(
            plan.first(),
            Some(&WorkspaceEdit::RenameFile(FileRenameEdit {
                old_path: project.clone(),
                new_path: renamed.clone(),
            }))
        );
        let mut path_edits = plan
            .into_iter()
            .filter_map(|edit| match edit {
                WorkspaceEdit::Text(edit) => {
                    let text = &ws.get(&edit.path).unwrap().text;
                    Some((
                        edit.path,
                        text[edit.edit.range].to_string(),
                        edit.edit.new_text,
                    ))
                }
                WorkspaceEdit::RenameFile(_) => None,
            })
            .collect::<Vec<_>>();
        path_edits.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            path_edits,
            vec![
                (
                    index.clone(),
                    "Project Plan.dj".to_string(),
                    "archive/Project Plan.dj".to_string()
                ),
                (
                    index,
                    "Project%20Plan.dj".to_string(),
                    "archive/Project Plan.dj".to_string()
                ),
                (
                    nested,
                    "../Project%20Plan.dj".to_string(),
                    "../archive/Project Plan.dj".to_string()
                ),
            ]
        );
    }

    #[test]
    fn workspace_reports_unresolved_references() {
        let a = PathBuf::from("/notes/a.dj");
        let b = PathBuf::from("/notes/b.dj");
        let doc_a = "# A\n\n[bad](#Missing) [file](missing.dj) [anchor](b.dj#Nope) [plain](AGENTS.md) [dir](crates/djot-core) [license](LICENSE) [ok](https://example.com)\n";
        let mut ws = Workspace::new();
        ws.insert(a.clone(), doc_a.to_string());
        ws.insert(b, "# Existing\n".to_string());

        let diagnostics = ws.diagnostics_for(&a);
        assert_eq!(diagnostics.len(), 3);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::UnresolvedAnchor {
                    id: "Missing".into(),
                }
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::UnresolvedPath {
                    path: "missing.dj".into(),
                }
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiagnosticKind::UnresolvedAnchor { id: "Nope".into() }
        }));
    }

    #[test]
    fn workspace_reports_invalid_recurring_task_metadata() {
        let path = PathBuf::from("/notes/tasks.dj");
        let doc = "{recur=\"P1W\"}\n::: task\nMissing due.\n:::\n\n{due=\"2026-06-21T09:00:00+08:00\" recur=\"P1M1D\"}\n::: task\nInvalid recur.\n:::\n\n{due=\"2026-06-21T09:00:00+08:00\" recur=\"P1W\"}\n::: task\nValid recur.\n:::\n";
        let mut ws = Workspace::new();
        ws.insert(path.clone(), doc.to_string());

        let diagnostics = ws.diagnostics_for(&path);
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::MissingTaskDueForRecur));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::InvalidTaskRecur {
                    recur: "P1M1D".into(),
                }
        }));
    }

    #[test]
    fn workspace_reports_conflicting_task_closed_state() {
        let path = PathBuf::from("/notes/tasks.dj");
        let doc = "{done=\"2026-06-21T09:00:00Z\" canceled=\"2026-06-21T10:00:00Z\"}\n::: task\nConflicting task.\n:::\n";
        let mut ws = Workspace::new();
        ws.insert(path.clone(), doc.to_string());

        let diagnostics = ws.diagnostics_for(&path);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].kind,
            DiagnosticKind::ConflictingTaskClosedState
        );
        assert_eq!(&doc[diagnostics[0].range.clone()], doc);
    }

    #[test]
    fn workspace_reports_task_prev_target_that_is_not_a_task() {
        let path = PathBuf::from("/notes/tasks.dj");
        let doc = "{#note}\nPlain anchor.\n\n{prev=\"#note\"}\n::: task\nFollow-up task.\n:::\n";
        let mut ws = Workspace::new();
        ws.insert(path.clone(), doc.to_string());

        let diagnostics = ws.diagnostics_for(&path);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].kind,
            DiagnosticKind::InvalidTaskPrevTarget { id: "note".into() }
        );
        assert_eq!(&doc[diagnostics[0].range.clone()], "#note");
    }

    #[test]
    fn workspace_accepts_task_prev_target_inherited_from_list_item() {
        let path = PathBuf::from("/notes/tasks.dj");
        let doc = "- {#previous-task}\n  ::: task\n  Previous task.\n  :::\n\n{prev=\"#previous-task\"}\n::: task\nFollow-up task.\n:::\n";
        let mut ws = Workspace::new();
        ws.insert(path.clone(), doc.to_string());

        assert_eq!(ws.diagnostics_for(&path), Vec::new());
    }

    #[test]
    fn workspace_resolves_task_dependencies_and_blocked_state() {
        let a = PathBuf::from("/notes/a.dj");
        let b = PathBuf::from("/notes/b.dj");
        let doc_a = "{#draft}\n::: task\nDraft.\n:::\n\n{#done done=\"2026-06-21T09:00:00Z\"}\n::: task\nDone.\n:::\n\n{#blocked depends=\"#draft b.dj#review\"}\n::: task\nBlocked.\n:::\n\n{#ready depends=\"#done\"}\n::: task\nReady.\n:::\n";
        let doc_b = "{#review}\n::: task\nReview.\n:::\n";
        let mut ws = Workspace::new();
        ws.insert(a.clone(), doc_a.to_string());
        ws.insert(b.clone(), doc_b.to_string());

        let blocked = ws.task_by_id(&a, "blocked").unwrap();
        let ready = ws.task_by_id(&a, "ready").unwrap();
        assert_eq!(
            ws.open_task_dependencies(&a, &blocked)
                .into_iter()
                .map(|dependency| dependency.target)
                .collect::<Vec<_>>(),
            vec![
                TaskRef {
                    path: a.clone(),
                    id: "draft".to_string(),
                },
                TaskRef {
                    path: b.clone(),
                    id: "review".to_string(),
                },
            ]
        );
        assert!(ws.is_task_blocked(&a, &blocked));
        assert!(!ws.is_task_blocked(&a, &ready));
        assert_eq!(
            ws.directly_blocking_tasks(&a, "draft"),
            vec![TaskRef {
                path: a.clone(),
                id: "blocked".to_string(),
            }]
        );
    }

    #[test]
    fn workspace_reports_invalid_task_dependencies() {
        let path = PathBuf::from("/notes/tasks.dj");
        let doc = "{#note}\nNot a task.\n\n{#missing-depends depends=\"#missing\"}\n::: task\nMissing.\n:::\n\n{#bare-depends depends=\"missing\"}\n::: task\nBare.\n:::\n\n{#non-task-depends depends=\"#note\"}\n::: task\nNon task.\n:::\n\n{#self-depends depends=\"#self-depends\"}\n::: task\nSelf.\n:::\n";
        let mut ws = Workspace::new();
        ws.insert(path.clone(), doc.to_string());

        let diagnostics = ws.diagnostics_for(&path);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::UnresolvedAnchor {
                    id: "missing".to_string(),
                }
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::InvalidTaskDependencyTarget {
                    target: "missing".to_string(),
                }
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::InvalidTaskDependencyTarget {
                    target: "#note".to_string(),
                }
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind
                == DiagnosticKind::TaskSelfDependency {
                    target: "#self-depends".to_string(),
                }
        }));
    }

    #[test]
    fn workspace_reports_dependency_cycles_and_blocked_tasks() {
        let path = PathBuf::from("/notes/tasks.dj");
        let doc =
            "{#a depends=\"#b\"}\n::: task\nA.\n:::\n\n{#b depends=\"#a\"}\n::: task\nB.\n:::\n";
        let mut ws = Workspace::new();
        ws.insert(path.clone(), doc.to_string());

        let diagnostics = ws.diagnostics_for(&path);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiagnosticKind::TaskDependencyCycle { id: "a".into() }
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiagnosticKind::TaskDependencyCycle { id: "b".into() }
        }));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.kind == DiagnosticKind::TaskBlocked { count: 1 } }));
    }

    #[test]
    fn workspace_reports_duplicate_anchors() {
        let path = PathBuf::from("/notes/tasks.dj");
        let doc =
            "{id=\"task\"}\n::: task\nFirst task.\n:::\n\n{id=task}\n::: task\nSecond task.\n:::\n";
        let mut ws = Workspace::new();
        ws.insert(path.clone(), doc.to_string());

        let diagnostics = ws.diagnostics_for(&path);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].kind,
            DiagnosticKind::DuplicateAnchor {
                id: "task".into(),
                first_range: 5..9,
            }
        );
        assert_eq!(&doc[diagnostics[0].range.clone()], "task");
    }

    #[test]
    fn repeat_rule_accepts_supported_iso_duration_subset() {
        assert_eq!(parse_repeat_rule("P1D"), Some(RepeatRule::Days(1)));
        assert_eq!(parse_repeat_rule("P2W"), Some(RepeatRule::Weeks(2)));
        assert_eq!(parse_repeat_rule("P1M"), Some(RepeatRule::Months(1)));
        assert_eq!(parse_repeat_rule("P1Y"), Some(RepeatRule::Years(1)));
        assert_eq!(parse_repeat_rule("P1M1D"), None);
        assert_eq!(parse_repeat_rule("PT1H"), None);
        assert_eq!(parse_repeat_rule("weekly"), None);
    }

    #[test]
    fn recur_due_supports_iso_week_duration() {
        let due = DateTime::parse_from_rfc3339("2026-06-21T17:00:00+08:00").unwrap();
        let next = next_recur_due(due, "P1W").unwrap();

        assert_eq!(next.to_rfc3339(), "2026-06-28T17:00:00+08:00");
    }

    #[test]
    fn recur_due_adds_calendar_months() {
        let due = DateTime::parse_from_rfc3339("2026-01-31T17:00:00+08:00").unwrap();
        let next = next_recur_due(due, "P1M").unwrap();

        assert_eq!(next.to_rfc3339(), "2026-02-28T17:00:00+08:00");
    }

    #[test]
    fn recur_due_adds_calendar_years() {
        let due = DateTime::parse_from_rfc3339("2024-02-29T17:00:00+08:00").unwrap();
        let next = next_recur_due(due, "P1Y").unwrap();

        assert_eq!(next.to_rfc3339(), "2025-02-28T17:00:00+08:00");
    }

    #[test]
    fn recur_due_rejects_composite_and_time_durations() {
        let due = DateTime::parse_from_rfc3339("2026-06-21T17:00:00+08:00").unwrap();

        assert!(next_recur_due(due, "P1M1D").is_none());
        assert!(next_recur_due(due, "PT1H").is_none());
        assert!(next_recur_due(due, "weekly").is_none());
    }

    #[test]
    fn anchor_attribute_uses_shorthand_only_for_ascii_name_ids() {
        assert_eq!(
            anchor_attribute("daily-review-2026-06-22"),
            "{#daily-review-2026-06-22}"
        );
        assert_eq!(
            anchor_attribute("学习-anki-2026-06-22"),
            "{id=\"学习-anki-2026-06-22\"}"
        );
        assert_eq!(
            anchor_attribute("quote\"backslash\\"),
            "{id=\"quote\\\"backslash\\\\\"}"
        );
    }

    #[test]
    fn recurring_attribute_filter_drops_instance_attribute_lines() {
        let source = "  {#task created=\"2026-06-21T00:00:00Z\" due=\"2026-06-22T00:00:00Z\" wait=\"2026-06-21T20:00:00Z\" recur=\"P1D\" done=\"2026-06-21T12:00:00Z\" canceled=\"2026-06-21T13:00:00Z\" prev=\"#old\"}\n  ::: task\n  Title\n  :::\n";

        assert_eq!(
            filter_recurring_instance_attributes(source),
            "  ::: task\n  Title\n  :::\n"
        );
    }

    #[test]
    fn recurring_attribute_filter_keeps_unknown_attribute_lines_verbatim() {
        let source = "  {project=\"anki\" priority=\"high\" .work}\n  ::: task\n  Title\n  :::\n";

        assert_eq!(filter_recurring_instance_attributes(source), source);
    }

    #[test]
    fn recurring_attribute_filter_rebuilds_mixed_attribute_lines() {
        let source = "  {project=\"anki cards\" recur=\"P1D\" priority=\"high\" #old}\n";

        assert_eq!(
            filter_recurring_instance_attributes(source),
            "  {project=\"anki cards\" priority=\"high\"}\n"
        );
    }

    #[test]
    fn recurring_attribute_filter_handles_quoted_spaces_and_escapes() {
        let source =
            "  {note=\"keep \\\"quoted\\\" value\" due=\"2026-06-22T00:00:00Z\" tag='two words'}\n";

        assert_eq!(
            filter_recurring_instance_attributes(source),
            "  {note=\"keep \\\"quoted\\\" value\" tag='two words'}\n"
        );
    }

    #[test]
    fn parse_dst_classifies_destinations() {
        assert_eq!(parse_dst("#sec"), RefTarget::Internal { id: "sec".into() });
        assert_eq!(
            parse_dst("mailto:a@b.c"),
            RefTarget::Url("mailto:a@b.c".into())
        );
        assert_eq!(
            parse_dst("other.dj"),
            RefTarget::External {
                path: "other.dj".into(),
                id: None
            }
        );
    }

    #[test]
    fn jotdown_cursor_link_parsing_shapes() {
        for (marked, expected_str) in [
            ("[|", Some("[")),
            ("[foo|", Some("[foo")),
            ("[foo|]", Some("[foo]")),
            ("[foo](|", Some("[foo](")),
            ("[foo](|)", None),
            ("[|]", Some("[]")),
        ] {
            let (text, cursor) = strip_cursor_marker(marked);
            assert_eq!(
                str_event_touching_cursor(&text, cursor).as_deref(),
                expected_str,
                "unexpected Str event at cursor for {marked:?}"
            );
        }

        let (text, cursor) = strip_cursor_marker("[foo](|)");
        assert!(
            Parser::new(&text).into_offset_iter().any(|(event, span)| {
                span.start <= cursor
                    && cursor <= span.end
                    && matches!(event, Event::End(Container::Link(_, _)))
            }),
            "cursor in a complete empty destination is in the link end syntax span"
        );
    }

    fn strip_cursor_marker(marked: &str) -> (String, usize) {
        let cursor = marked.find('|').expect("cursor marker");
        (marked.replace('|', ""), cursor)
    }

    fn str_event_touching_cursor(text: &str, cursor: usize) -> Option<String> {
        Parser::new(text)
            .into_offset_iter()
            .find_map(|(event, span)| match event {
                Event::Str(s) if span.start <= cursor && cursor <= span.end => Some(s.to_string()),
                _ => None,
            })
    }
}
