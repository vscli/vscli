//! Native file-participant review policy and immutable full-edit overview.
//! The overview grants no edit authority. Worker-only range preview may load
//! bounded local baselines; native publication belongs to the App coordinator.
pub mod dialog;
pub mod preferences;
pub(crate) mod preview;
use crate::workspace_edit_stream::{Entry, FileKind, Stream};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Initial {
    NoChanges,
    Apply { show_preview: bool },
    Ask { requires_preview: bool },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Apply,
    Preview,
    Skip,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Skip,
    Apply {
        show_preview: bool,
        remember: Option<bool>,
    },
}
/// Pinned MainThreadFileSystemEventService's initial decision, before bulk-edit
/// preview/annotation policy. A stored choice bypasses this particular dialog.
pub fn initial(has_changes: bool, needs_confirmation: bool, remembered: Option<bool>) -> Initial {
    if !has_changes {
        Initial::NoChanges
    } else if let Some(show_preview) = remembered {
        Initial::Apply { show_preview }
    } else {
        Initial::Ask {
            requires_preview: needs_confirmation,
        }
    }
}
pub fn choose(initial: Initial, choice: Choice, remember: bool) -> Result<Decision> {
    let Initial::Ask { requires_preview } = initial else {
        anyhow::bail!("No file-participant choice is pending");
    };
    if choice == Choice::Skip {
        return Ok(Decision::Skip);
    }
    ensure!(
        !requires_preview || choice == Choice::Preview,
        "Confirmation requires preview before applying edits"
    );
    let show_preview = choice == Choice::Preview;
    Ok(Decision::Apply {
        show_preview,
        remember: (!requires_preview && remember).then_some(show_preview),
    })
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Create,
    Rename,
    Copy,
    Delete,
    Text,
    Snippet,
    EndOfLine,
    TextAndEndOfLine,
    Notebook,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub index: usize,
    pub change: Change,
    pub resource: String,
    pub destination: Option<String>,
    pub label: Option<String>,
    pub needs_confirmation: bool,
}
pub struct Overview {
    original: Arc<Value>,
    rows: Vec<Row>,
    needs_confirmation: bool,
}
impl Overview {
    /// Invoke on a background worker. Keep the original DTO alive; summaries
    /// never replace/truncate file options, text, snippets, EOL or notebook data.
    pub fn prepare(original: Arc<Value>) -> Result<Self> {
        let stream = Stream::parse(&original)?;
        let mut rows = Vec::new();
        rows.try_reserve_exact(stream.len())?;
        let mut needs_confirmation = false;
        for index in 0..stream.len() {
            let entry = stream.entry(index)?;
            let (change, resource, destination, metadata) = match entry {
                Entry::File(file) => {
                    let change = match file.kind() {
                        FileKind::Create => Change::Create,
                        FileKind::Rename => Change::Rename,
                        FileKind::Copy => Change::Copy,
                        FileKind::Delete => Change::Delete,
                    };
                    let resource = file
                        .old_resource
                        .or(file.new_resource)
                        .expect("validated resource");
                    let destination = file.old_resource.and(file.new_resource).map(str::to_owned);
                    (change, resource, destination, file.metadata)
                }
                Entry::Text(text) => {
                    let change = if text
                        .edit
                        .get("insertAsSnippet")
                        .is_some_and(crate::workspace_edit_stream::truthy)
                    {
                        Change::Snippet
                    } else if text.edit.contains_key("eol") {
                        let empty = text.edit.get("text").and_then(Value::as_str) == Some("");
                        let point = text.edit.get("range").is_none_or(|range| {
                            range["startLineNumber"] == 1
                                && range["startColumn"] == 1
                                && range["endLineNumber"] == 1
                                && range["endColumn"] == 1
                        });
                        if empty && point {
                            Change::EndOfLine
                        } else {
                            Change::TextAndEndOfLine
                        }
                    } else {
                        Change::Text
                    };
                    (change, text.resource, None, text.metadata)
                }
                Entry::Notebook(notebook) => {
                    (Change::Notebook, notebook.resource, None, notebook.metadata)
                }
            };
            let confirmation = metadata
                .and_then(|metadata| metadata.get("needsConfirmation"))
                .is_some_and(crate::workspace_edit_stream::truthy);
            needs_confirmation |= confirmation;
            rows.push(Row {
                index,
                change,
                resource: resource.into(),
                destination,
                label: metadata
                    .and_then(|metadata| metadata.get("label"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                needs_confirmation: confirmation,
            });
        }
        Ok(Self {
            original,
            rows,
            needs_confirmation,
        })
    }
    pub fn original(&self) -> &Arc<Value> {
        &self.original
    }
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
    pub fn needs_confirmation(&self) -> bool {
        self.needs_confirmation
    }
    pub fn initial(&self, remembered: Option<bool>) -> Initial {
        initial(!self.rows.is_empty(), self.needs_confirmation, remembered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn source_initial_choice_keeps_remembered_policy_separate_from_lower_preview_requirements() {
        for needed in [false, true] {
            for remembered in [None, Some(false), Some(true)] {
                assert_eq!(initial(false, needed, remembered), Initial::NoChanges);
                assert_eq!(
                    initial(true, needed, remembered),
                    remembered.map_or(
                        Initial::Ask {
                            requires_preview: needed
                        },
                        |show_preview| Initial::Apply { show_preview }
                    )
                );
            }
        }
    }
    #[test]
    fn source_skip_never_persists_and_confirmation_cannot_choose_direct_application() {
        for required in [false, true] {
            let offer = Initial::Ask {
                requires_preview: required,
            };
            for remember in [false, true] {
                assert_eq!(
                    choose(offer, Choice::Skip, remember).unwrap(),
                    Decision::Skip
                );
                let preview = choose(offer, Choice::Preview, remember).unwrap();
                assert_eq!(
                    preview,
                    Decision::Apply {
                        show_preview: true,
                        remember: (!required && remember).then_some(true)
                    }
                );
                if required {
                    assert!(choose(offer, Choice::Apply, remember).is_err());
                } else {
                    assert_eq!(
                        choose(offer, Choice::Apply, remember).unwrap(),
                        Decision::Apply {
                            show_preview: false,
                            remember: remember.then_some(false)
                        }
                    );
                }
            }
        }
        for initial in [
            Initial::NoChanges,
            Initial::Apply {
                show_preview: false,
            },
            Initial::Apply { show_preview: true },
        ] {
            assert!(choose(initial, Choice::Preview, true).is_err());
        }
    }
    #[test]
    fn overview_keeps_original_all_kinds_order_binary_options_annotations_and_versions() {
        let original = Arc::new(json!({"edits":[
            {"newResource":"file:///new","options":{"contents":{"type":"base64","value":"AAH/"},"overwrite":false}},
            {"oldResource":"file:///old","newResource":"file:///moved","metadata":{"label":"move 猫🙂","needsConfirmation":true}},
            {"oldResource":"file:///old","newResource":"file:///copy","options":{"copy":true}},
            {"oldResource":"file:///deleted","options":{"recursive":true}},
            {"resource":"file:///text","versionId":7,"textEdit":{"text":"猫🙂\r\n","range":{"startLineNumber":1,"startColumn":1,"endLineNumber":1,"endColumn":1}}},
            {"resource":"file:///snippet","textEdit":{"text":"${1:name}","insertAsSnippet":true}},
            {"resource":"file:///eol","textEdit":{"text":"","eol":1}},
            {"resource":"notebook:///a","cellEdit":{"editType":5,"metadata":{"exact":"preserved"}}}
        ]}));
        let model = Overview::prepare(original.clone()).unwrap();
        assert!(Arc::ptr_eq(model.original(), &original));
        assert_eq!(model.rows().len(), 8);
        assert_eq!(
            model
                .rows()
                .iter()
                .map(|row| row.change.clone())
                .collect::<Vec<_>>(),
            [
                Change::Create,
                Change::Rename,
                Change::Copy,
                Change::Delete,
                Change::Text,
                Change::Snippet,
                Change::EndOfLine,
                Change::Notebook
            ]
        );
        assert_eq!(
            model.rows().iter().map(|row| row.index).collect::<Vec<_>>(),
            (0..8).collect::<Vec<_>>()
        );
        assert_eq!(model.rows()[1].label.as_deref(), Some("move 猫🙂"));
        assert_eq!(
            model.rows()[1].destination.as_deref(),
            Some("file:///moved")
        );
        assert!(model.needs_confirmation());
        assert_eq!(
            model.original()["edits"][0]["options"]["contents"]["value"],
            "AAH/"
        );
        assert_eq!(model.original()["edits"][4]["versionId"], 7);
    }
    #[test]
    fn confirmation_metadata_uses_source_javascript_truthiness_without_rewriting_values() {
        for (value, expected) in [
            (json!(null), false),
            (json!(false), false),
            (json!(0), false),
            (json!(""), false),
            (json!("false"), true),
            (json!([]), true),
            (json!({}), true),
            (json!(1), true),
        ] {
            let data = Arc::new(
                json!({"edits":[{"resource":"file:///a","textEdit":{"text":"a"},"metadata":{"needsConfirmation":value}}]}),
            );
            let review = Overview::prepare(data.clone()).unwrap();
            assert_eq!(review.needs_confirmation(), expected);
            assert_eq!(
                review.original()["edits"][0]["metadata"]["needsConfirmation"],
                data["edits"][0]["metadata"]["needsConfirmation"]
            );
        }
    }
    #[test]
    fn malformed_later_row_and_excess_resources_refuse_whole_review() {
        assert!(Overview::prepare(Arc::new(json!({"edits":[{"newResource":"file:///a"},{"textEdit":{"text":"missing resource"}}]}))).is_err());
        assert!(Overview::prepare(Arc::new(json!({"edits":(0..129).map(|index|json!({"newResource":format!("file:///{index}")})).collect::<Vec<_>>()}))).is_err());
    }
    #[test]
    fn source_metadata_depth_survives_overview_and_excessive_native_depth_is_bounded() {
        let mut nested = json!("leaf");
        for _ in 0..64 {
            nested = json!({"nested":nested});
        }
        let data = Arc::new(
            json!({"edits":[{"resource":"file:///a","textEdit":{"text":"a"},"metadata":{"custom":nested}}]}),
        );
        assert!(Overview::prepare(data).is_ok());
        let mut nested = json!("leaf");
        for _ in 0..97 {
            nested = json!({"nested":nested});
        }
        let data = Arc::new(
            json!({"edits":[{"resource":"file:///a","textEdit":{"text":"a"},"metadata":{"custom":nested}}]}),
        );
        assert!(Overview::prepare(data).is_err());
    }
}
