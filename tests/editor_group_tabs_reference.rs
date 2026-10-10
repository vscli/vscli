//! Pure membership engine against independently captured public VS Code groups.
//!
//! This projection compares order/focus/navigation/close and document references.
//! Text, caret/view restoration, dirty prompts, native Document identity/history,
//! filesystem receipts and terminal geometry belong to separate App qualification.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use vscli::editor_groups::{Groups, Navigate};

const ROOT: &str = "tests/vscode-reference";
const COMMIT: &str = "912bb683695358a54ae0c670461738984cbb5b95";
const RESOURCES: [&str; 4] = ["a.txt", "b.txt", "c.txt", "d.txt"];
const SOURCES: [&str; 9] = [
    "editor-group-tabs.cjs",
    "editor-group-tabs-cases.json",
    "editor-group-tabs-suite.cjs",
    "editor-group-tabs-run.cjs",
    "editor-group-tabs-worker.cjs",
    "supervisor.cjs",
    "package-lock.json",
    "package.json",
    "extension.cjs",
];
fn bytes(path: &Path) -> Vec<u8> {
    assert!(fs::metadata(path).unwrap().len() <= 16 * 1024 * 1024);
    let bytes = fs::read(path).unwrap();
    assert!(bytes.len() <= 16 * 1024 * 1024);
    bytes
}
fn source(name: &str) -> Vec<u8> {
    assert_eq!(Path::new(name).components().count(), 1);
    bytes(&Path::new(env!("CARGO_MANIFEST_DIR")).join(ROOT).join(name))
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn resource(id: u64) -> &'static str {
    RESOURCES[(id - 1) as usize]
}
fn document(resource: &str) -> u64 {
    RESOURCES.iter().position(|name| *name == resource).unwrap() as u64 + 1
}
fn without_action(observation: &Value) -> Value {
    let mut observed = observation.clone();
    let object = observed.as_object_mut().unwrap();
    object.remove("action");
    object.remove("opened");
    observed
}
fn corpus() -> (Vec<Value>, Vec<Value>) {
    let (directory, stem) = match std::env::var_os("VSCLI_EDITOR_GROUP_TABS_REFERENCE_DIR") {
        Some(directory) => (PathBuf::from(directory), "editor-group-tabs"),
        None => (
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(ROOT)
                .join("baselines/1.95.0/editor-group-tabs"),
            "linux",
        ),
    };
    let trace = bytes(&directory.join(format!("{stem}.json")));
    let evidence = bytes(&directory.join(format!("{stem}-evidence.json")));
    let proof: Value =
        serde_json::from_slice(&bytes(&directory.join(format!("{stem}-provenance.json")))).unwrap();
    let case_bytes = source("editor-group-tabs-cases.json");
    let cases: Vec<Value> = serde_json::from_slice(&case_bytes).unwrap();
    let rows: Vec<Value> = serde_json::from_slice(&trace).unwrap();
    let raw: Vec<Value> = serde_json::from_slice(&evidence).unwrap();
    assert_eq!(proof["version"], "1.95.0");
    assert_eq!(proof["commit"], COMMIT);
    assert_eq!(proof["caseCount"], 8);
    assert_eq!(proof["snapshotCount"], 70);
    assert_eq!(proof["traceSha256"], sha(&trace));
    assert_eq!(proof["evidenceSha256"], sha(&evidence));
    assert_eq!(proof["casesSha256"], sha(&case_bytes));
    assert_eq!(cases.len(), 8);
    assert_eq!(rows.len(), cases.len());
    assert_eq!(raw.len(), cases.len());
    let runs = proof["runs"].as_array().unwrap();
    assert_eq!(runs.len(), cases.len());
    let policy = json!({
        "workbench.editor.enablePreview":false,
        "workbench.editor.enablePreviewFromQuickOpen":false,
        "workbench.editor.openPositioning":"right",
        "workbench.editor.focusRecentEditorAfterClose":true,
        "workbench.editor.closeEmptyGroups":true,
        "workbench.editor.revealIfOpen":false
    });
    let mut observations = 0;
    for (((case, row), raw), run) in cases.iter().zip(&rows).zip(&raw).zip(runs) {
        assert_eq!(row["name"], case["name"]);
        assert_eq!(row["name"], raw["name"]);
        assert_eq!(row["name"], run["name"]);
        assert_eq!(run["version"], "1.95.0");
        assert_eq!(run["commit"], COMMIT);
        let hashes = run["sources"].as_object().unwrap();
        assert_eq!(hashes.len(), SOURCES.len());
        for file in SOURCES {
            assert_eq!(
                hashes[file],
                sha(&source(file)),
                "Reference source changed: {file}"
            );
        }
        // Both writers use two-space JSON plus a final newline, preserve field
        // order, and retain raw evidence. Validate each individual run too.
        let mut encoded = serde_json::to_vec_pretty(raw).unwrap();
        encoded.push(b'\n');
        assert_eq!(run["evidenceSha256"], sha(&encoded));
        assert_eq!(raw["setup"]["effectiveConfiguration"], policy);
        assert_eq!(
            raw["setup"]["action"],
            "api.openTextDocument/showTextDocument"
        );
        assert_eq!(raw["setup"]["resource"], "a.txt");
        let steps = case["steps"].as_array().unwrap();
        let mut required = vec![json!("vscode.open")];
        for step in steps {
            if let Some(command) = step.get("command")
                && !required.contains(command)
            {
                required.push(command.clone());
            }
        }
        assert_eq!(raw["setup"]["commandInventory"], json!(required));
        assert_eq!(row["observations"], raw["observations"]);
        let snapshots = row["observations"].as_array().unwrap();
        let settlements = raw["settlements"].as_array().unwrap();
        let details = raw["details"].as_array().unwrap();
        assert_eq!(snapshots.len(), steps.len() + 1);
        assert_eq!(settlements.len(), snapshots.len());
        assert_eq!(details.len(), steps.len());
        assert_eq!(snapshots[0]["action"], "initial");
        assert_eq!(raw["setup"]["settlement"], settlements[0]);
        for (snapshot, settlement) in snapshots.iter().zip(settlements) {
            assert_eq!(without_action(snapshot), settlement["observed"]);
            for group in snapshot["groups"].as_array().unwrap() {
                for tab in group["tabs"].as_array().unwrap() {
                    assert_eq!(tab["preview"], false);
                    assert_eq!(tab["pinned"], false, "Committed tabs are not sticky pins");
                }
            }
            for (index, observed) in snapshot["documents"].as_array().unwrap().iter().enumerate() {
                assert_eq!(observed["resource"], RESOURCES[index]);
                assert_eq!(observed["disk"], snapshots[0]["documents"][index]["disk"]);
            }
        }
        for (index, (step, detail)) in steps.iter().zip(details).enumerate() {
            assert_eq!(detail["step"], index);
            assert_eq!(detail["gesture"], *step);
            if let Some(open) = step.get("open") {
                assert_eq!(snapshots[index + 1]["action"], "vscode.open");
                assert_eq!(snapshots[index + 1]["opened"], *open);
            } else {
                assert_eq!(snapshots[index + 1]["action"], step["command"]);
            }
            if step["requireSharedDirty"] == true {
                let previous = &snapshots[index];
                assert_eq!(previous["active"]["dirty"], true);
                let resource = &previous["active"]["resource"];
                assert!(
                    previous["groups"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .flat_map(|group| group["tabs"].as_array().unwrap())
                        .filter(|tab| &tab["resource"] == resource)
                        .count()
                        >= 2
                );
            }
        }
        observations += snapshots.len();
    }
    assert_eq!(observations, 70);
    let files = proof["files"].as_array().unwrap();
    assert_eq!(files.len(), 32);
    for (index, case) in cases.iter().enumerate() {
        for (resource_index, name) in RESOURCES.iter().enumerate() {
            let file = &files[index * 4 + resource_index];
            assert_eq!(file["case"], case["name"]);
            assert_eq!(file["resource"], *name);
            assert_eq!(
                file["sha256"],
                sha(
                    rows[index]["observations"][0]["documents"][resource_index]["disk"]
                        .as_str()
                        .unwrap()
                        .as_bytes()
                )
            );
        }
    }
    (cases, rows)
}

fn compare(groups: &Groups, snapshot: &Value, context: &str) -> bool {
    let expected = snapshot["groups"].as_array().unwrap();
    if snapshot["active"].is_null() {
        // This is an explicitly asserted representation boundary, not a
        // normalized group-geometry match: VS Code keeps its last empty group.
        assert_eq!(
            snapshot["groups"],
            json!([{"viewColumn":1,"active":true,"tabs":[]}])
        );
        assert_eq!(snapshot["visible"], json!([]));
        assert!(
            groups.groups().is_empty(),
            "{context}: native welcome has zero groups"
        );
        assert_eq!(groups.active_group(), None);
        assert_eq!(groups.active_membership(), None);
        return true;
    }
    assert_eq!(groups.groups().len(), expected.len(), "{context}");
    for (index, (native, upstream)) in groups.groups().iter().zip(expected).enumerate() {
        assert_eq!(
            upstream["viewColumn"],
            index + 1,
            "{context}: ordered public columns"
        );
        assert_eq!(
            groups.active_group() == Some(native.id()),
            upstream["active"].as_bool().unwrap(),
            "{context}: group focus"
        );
        let actual: Vec<_> = native.tabs().iter().map(|tab| json!({
            "resource":resource(tab.document()), "active":native.active().unwrap().id()==tab.id()
        })).collect();
        let expected: Vec<_> = upstream["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tab| json!({"resource":tab["resource"],"active":tab["active"]}))
            .collect();
        assert_eq!(
            actual, expected,
            "{context}: exact ordered memberships and active tab"
        );
    }
    let active = groups.active_membership().unwrap();
    assert_eq!(
        resource(active.document),
        snapshot["active"]["resource"].as_str().unwrap(),
        "{context}: active resource"
    );
    for name in RESOURCES {
        let expected = expected
            .iter()
            .flat_map(|group| group["tabs"].as_array().unwrap())
            .filter(|tab| tab["resource"] == name)
            .count();
        let memberships: Vec<_> = groups.memberships(document(name)).collect();
        assert_eq!(
            memberships.len(),
            expected,
            "{context}: shared reference count for {name}"
        );
        assert!(
            memberships
                .iter()
                .all(|member| member.document == document(name))
        );
    }
    let visible = snapshot["visible"].as_array().unwrap();
    assert_eq!(
        visible.len(),
        groups.groups().len(),
        "{context}: visible active memberships"
    );
    for (native, editor) in groups.groups().iter().zip(visible) {
        assert_eq!(
            resource(native.active().unwrap().document()),
            editor["resource"].as_str().unwrap(),
            "{context}: visible resource"
        );
    }
    // Public reference identity is preserved as evidence of shared models.
    // The engine's stable integer references are not native Document allocation.
    for first in visible {
        for second in visible {
            if first["resource"] == second["resource"] {
                assert_eq!(
                    first["documentObject"], second["documentObject"],
                    "{context}: public shared TextDocument"
                );
            }
        }
    }
    false
}

#[test]
fn ordered_memberships_focus_split_navigation_and_mru_match_eight_pinned_cases() {
    let (cases, rows) = corpus();
    let mut compared = 0;
    let mut welcome_boundaries = 0;
    for (case, row) in cases.iter().zip(&rows) {
        let name = case["name"].as_str().unwrap();
        let observations = row["observations"].as_array().unwrap();
        let mut groups = Groups::default();
        groups.open(document("a.txt")).unwrap();
        welcome_boundaries += usize::from(compare(&groups, &observations[0], name));
        compared += 1;
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            if let Some(open) = step.get("open") {
                groups.open(document(open.as_str().unwrap())).unwrap();
            } else {
                match step["command"].as_str().unwrap() {
                    "workbench.action.nextEditor" => {
                        groups.navigate(Navigate::Next).unwrap();
                    }
                    "workbench.action.previousEditor" => {
                        groups.navigate(Navigate::Previous).unwrap();
                    }
                    "workbench.action.nextEditorInGroup" => {
                        groups.navigate(Navigate::NextInGroup).unwrap();
                    }
                    "workbench.action.previousEditorInGroup" => {
                        groups.navigate(Navigate::PreviousInGroup).unwrap();
                    }
                    "workbench.action.splitEditor" => {
                        groups.split_active().unwrap();
                    }
                    "workbench.action.focusFirstEditorGroup" => {
                        groups.focus_group(groups.groups()[0].id()).unwrap();
                    }
                    "workbench.action.focusSecondEditorGroup" => {
                        groups.focus_group(groups.groups()[1].id()).unwrap();
                    }
                    "workbench.action.closeActiveEditor" => {
                        groups.close(groups.active_membership().unwrap()).unwrap();
                    }
                    "workbench.action.closeEditorsInGroup" => {
                        let proof = groups.group_proof(groups.active_group().unwrap()).unwrap();
                        groups.close_group(&proof).unwrap();
                    }
                    // These real reference targets change documents/views, not
                    // membership. Their membership projection must stay stable;
                    // this consumer does not implement or claim their behavior.
                    "cursorRight" | "type" | "undo" => {}
                    command => panic!("Unqualified engine gesture: {command}"),
                }
            }
            welcome_boundaries += usize::from(compare(
                &groups,
                &observations[index + 1],
                &format!("{name} step {index}"),
            ));
            compared += 1;
        }
    }
    assert_eq!(compared, 70);
    assert_eq!(
        welcome_boundaries, 3,
        "Explicit upstream one-empty-group/native zero-group boundaries"
    );
    eprintln!(
        "Qualified 8 cases / 70 membership snapshots; 3 explicit upstream-one-empty/native-zero welcome boundaries. Caret/text/dirty/history/geometry remain App qualification."
    );
}
