//! Metadata-only admission for exact StableV2 artifacts. No native workflow or
//! default-baseline selection is authorized by this module.
use super::*;
use anyhow::{Result, ensure};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use std::{collections::BTreeSet, fmt, io::Read};

const REVISION: &str = "133897055d5c524ed1fd0b51866edf95f1faffd8";
const INPUT: &str = "tests/vscode-reference-save-actions-stable-configuration-candidate";
const OUTPUT: &str = "target/save-actions-stable-configuration";
const RAW_SCOPE: &str = "Pinned synthetic sourceActions, layered/language settings and original Save/Undo/Redo; exact callback order and text/disk/selection. Not native LSP, unchanged extension or full save-lifecycle parity";
const READY_SCOPE: &str = "Public source-defined canonical resource/language configuration before original target commands; no settings update, private migration forcing, target-output predicate or target retry";
const AUX_SCOPE: &str = "Separate explicit public auxiliary Save and exact null-formatter callback establish save-contribution installation for every target reason; no target output predicate or target command retry";
const SETUP_SCOPE: &str = "One public execute-source-provider call; no edits applied, exact unchanged proof and no readiness retry";
const STABLE_SCOPE: &str = "Canonical configuration checks only; all original target outputs and events remain retained even on mismatch";
const STEP_SCOPE: &str = "Actual command/didSave acknowledgement followed by minimum100ms/50ms stable public state; no preferred target-output predicate or retry";
const IO_SCOPE: &str = "Declared launch bytes and actual post-editor settings bytes; upstream migration may change runtime files; no target-outcome retry";
const FAILURE_SCOPE: &str = "Actual bounded failed setup prefix; target commands were not started unless targetStarted is true; no outcome retry";
const FAILURE_PROVENANCE_SCOPE: &str = "Actual failed setup/target prefix preserved before rethrow; no successful target qualification claimed";
const PREFIX_SCOPE: &str = "Actual issued/pending gesture and completed public target prefix on failure; no successful target qualification";
const PARTIAL_SCOPE: &str =
    "Exact retained diagnostic prefix on failed setup; not a successful target capture";
const TEXT: &str = "fix=0\r\nimports=0\r\nchild=0\r\nother=0\r\n猫🙂\r\n";
const MIB: usize = 1024 * 1024;
const SOURCES: &[(&str, &str)] = &[
    (
        "save-code-actions.cjs",
        "918045d167f724cbb7ba63025d1bb5896ea02425808ede65b12d3547e6ad19be",
    ),
    (
        "save-configuration-ready.cjs",
        "c98c112a06a7d7db6a37833a695b1b10e7b3518bb84ec6a77bd29d0403088aca",
    ),
    (
        "save-code-actions-cases.json",
        "a96e7078cd7e710cb33fc0c3d5e4e40e9cd0283e884035ff552dfd37b1a9be9c",
    ),
    (
        "save-code-actions-suite.cjs",
        "e042514f0fb763a86341e0a361d8ec68abdfba112343a208b57a6be80d9cf11f",
    ),
    (
        "save-code-actions-run.cjs",
        "81e11f58262d2340ec79d46ec175130078b77031999ef46fecb31f0ab5d98b03",
    ),
    (
        "save-code-actions-worker.cjs",
        "62a483ea66001c814cd2e2f5bd64cb9e37862b005023552953d631e8a5631ecf",
    ),
    (
        "supervisor.cjs",
        "d0bb864b1b699fe29d6b4e24de7a1fa587fe122b6b4a92e9c74cfd25a74d5b22",
    ),
    (
        "package-lock.json",
        "74d30a58472daabec49f33d54bc1bfd1b9fb8d0858eec22a022c2464c263ca8c",
    ),
    (
        "package.json",
        "4358149fa583c7a8402d0396e06bb88b1ddb6f5ca82c2002c4b841da1a14bec3",
    ),
    (
        "extension.cjs",
        "64115fcd8cfbe8fc111970b4f536dff54e505f92625dbce117e2cd60e4490c75",
    ),
];

// Duplicate keys must not disappear before exact inventory/schema checks.
struct Unique(Value);
impl<'de> serde::Deserialize<'de> for Unique {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("bounded JSON with unique object keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Unique, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Unique(Value::Number(n)))
                    .ok_or_else(|| E::custom("Non-finite JSON"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::String(v)))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> std::result::Result<Unique, E> {
                self.visit_unit()
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut out = Vec::new();
                while let Some(Unique(v)) = a.next_element()? {
                    if out.len() >= 4096 {
                        return Err(de::Error::custom("JSON array budget exceeded"));
                    }
                    out.push(v);
                }
                Ok(Unique(Value::Array(out)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut out = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if out.len() >= 4096 || out.contains_key(&k) {
                        return Err(de::Error::custom("Duplicate or oversized JSON map"));
                    }
                    let Unique(v) = a.next_value()?;
                    out.insert(k, v);
                }
                Ok(Unique(Value::Object(out)))
            }
        }
        d.deserialize_any(V)
    }
}
fn parse(bytes: &[u8]) -> Result<Value> {
    Ok(serde_json::from_slice::<Unique>(bytes)?.0)
}
fn bounded(path: &Path, cap: usize) -> Result<Vec<u8>> {
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_file(),
        "StableV2 file budget exceeded"
    );
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let f = options.open(path)?;
    let before = f.metadata()?;
    ensure!(
        before.is_file() && before.len() <= cap as u64,
        "StableV2 file budget exceeded"
    );
    let mut out = Vec::with_capacity(before.len() as usize);
    let after = f.try_clone()?;
    f.take(cap as u64 + 1).read_to_end(&mut out)?;
    let end = after.metadata()?;
    ensure!(
        out.len() <= cap
            && out.len() as u64 == before.len()
            && end.len() == before.len()
            && end.modified()? == before.modified()?,
        "StableV2 file changed during read"
    );
    Ok(out)
}
fn inventory(path: &Path, cap: usize) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    for entry in fs::read_dir(path)? {
        ensure!(
            out.len() < cap,
            "StableV2 directory inventory exceeds bounds"
        );
        let name = entry?
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("StableV2 non-UTF8 inventory"))?;
        ensure!(
            name.len() <= 256 && out.insert(name),
            "StableV2 duplicate inventory"
        );
    }
    Ok(out)
}
fn read(path: &Path, cap: usize) -> Result<Value> {
    parse(&bounded(path, cap)?)
}
fn keys(v: &Value, required: &[&str]) -> Result<()> {
    let m = v
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("StableV2 shape changed"))?;
    ensure!(
        m.len() == required.len() && required.iter().all(|k| m.contains_key(*k)),
        "StableV2 shape changed"
    );
    Ok(())
}
fn optional(v: &Value, required: &[&str], permitted: &[&str]) -> Result<()> {
    let m = v
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("StableV2 shape changed"))?;
    ensure!(
        required.iter().all(|k| m.contains_key(*k))
            && m.keys()
                .all(|k| required.contains(&k.as_str()) || permitted.contains(&k.as_str())),
        "StableV2 shape changed"
    );
    Ok(())
}
fn list(v: &Value, cap: usize) -> Result<&[Value]> {
    let a = v
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("StableV2 array changed"))?;
    ensure!(a.len() <= cap, "StableV2 record budget exceeded");
    Ok(a)
}
fn integer(v: &Value, cap: u64) -> Result<u64> {
    v.as_u64()
        .filter(|n| *n <= cap)
        .ok_or_else(|| anyhow::anyhow!("StableV2 integer changed"))
}
fn text(v: &Value, cap: usize) -> Result<&str> {
    v.as_str()
        .filter(|s| s.len() <= cap)
        .ok_or_else(|| anyhow::anyhow!("StableV2 string budget exceeded"))
}
fn error_text(v: &Value, cap: usize) -> Result<()> {
    ensure!(
        v.as_str().is_some_and(|s| s.encode_utf16().count() <= cap),
        "StableV2 error budget exceeded"
    );
    Ok(())
}
fn boolean(v: &Value) -> Result<bool> {
    v.as_bool()
        .ok_or_else(|| anyhow::anyhow!("StableV2 boolean changed"))
}
fn digest(v: &Value) -> Result<()> {
    ensure!(
        v.as_str().is_some_and(|s| s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))),
        "StableV2 digest changed"
    );
    Ok(())
}
fn compact(v: &Value, cap: usize) -> Result<()> {
    struct Count {
        n: usize,
        cap: usize,
    }
    impl std::io::Write for Count {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.n = self
                .n
                .checked_add(b.len())
                .ok_or_else(|| std::io::Error::other("JSON budget exceeded"))?;
            if self.n > self.cap {
                return Err(std::io::Error::other("JSON budget exceeded"));
            }
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Count { n: 0, cap }, v)
        .map_err(|_| anyhow::anyhow!("StableV2 compact byte budget exceeded"))?;
    Ok(())
}
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((ak, av), (bk, bv))| ak == bk && same(av, bv))
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
        }
        _ => a == b,
    }
}
fn actions(fixture: &Value) -> Value {
    match fixture["name"].as_str().unwrap() {
        "object-fixall-first" => {
            json!({"source.organizeImports":"explicit","source.fixAll":"explicit"})
        }
        "array-imports-first" | "after-delay-array-skipped" => {
            json!(["source.organizeImports", "source.fixAll"])
        }
        "array-fixall-first" => json!(["source.fixAll", "source.organizeImports"]),
        "ancestor-false-child" | "ancestor-never-child" => {
            json!({"source.fixAll":"explicit","source.fixAll.child":"never"})
        }
        "source-ancestor-false-family" | "source-ancestor-never-family" => {
            json!({"source":"explicit","source.fixAll":"never"})
        }
        "after-delay-always-skipped" => {
            json!({"source.fixAll":"always","source.organizeImports":"always"})
        }
        "user-workspace-object-merge" => {
            json!({"source.fixAll":"explicit","source.organizeImports":"explicit"})
        }
        "language-composite-single-merge" => {
            json!({"source.organizeImports":"explicit","source.fixAll":"explicit","source.fixAll.child":"never"})
        }
        "workspace-array-replaces-object" => json!(["source.organizeImports"]),
        "workspace-empty-object-retains-user" => json!({"source.fixAll":"explicit"}),
        "child-only-dot-boundary" => json!({"source.fixAll.child":"explicit"}),
        _ => unreachable!("only source-qualified compiled fixtures"),
    }
}
fn canonical(v: &Value) -> Value {
    if v.is_array() {
        return v.clone();
    }
    let mut out = serde_json::Map::new();
    for (k, v) in v.as_object().unwrap() {
        out.insert(
            k.clone(),
            match v.as_bool() {
                Some(true) => json!("explicit"),
                Some(false) => json!("never"),
                None => v.clone(),
            },
        );
    }
    Value::Object(out)
}
fn expected(f: &Value) -> Value {
    let mut a = json!({"key":"editor.codeActionsOnSave","defaultValue":{},"globalValue":canonical(&f["user"]["editor.codeActionsOnSave"])});
    if let Some(v) = f["workspace"].get("editor.codeActionsOnSave") {
        a["workspaceValue"] = canonical(v);
        a["workspaceFolderValue"] = canonical(v);
    }
    if f["name"] == "language-composite-single-merge" {
        a["globalLanguageValue"] = json!({"source.fixAll":"explicit"});
        a["workspaceLanguageValue"] = json!({"source.fixAll.child":"never"});
        a["workspaceFolderLanguageValue"] = json!({"source.fixAll.child":"never"});
        a["languageIds"] = json!(["plaintext", "cpp"]);
    }
    json!({"effective":{"codeActionsOnSave":actions(f),"formatOnSave":false,"autoSave":f["autosave"],"autoSaveDelay":500},
        "inspect":{"codeActionsOnSave":a,"formatOnSave":{"key":"editor.formatOnSave","defaultValue":false,"globalValue":false,"languageIds":["ini"]},
        "autoSave":{"key":"files.autoSave","defaultValue":"off","globalValue":f["autosave"]},
        "autoSaveDelay":{"key":"files.autoSaveDelay","defaultValue":1000,"globalValue":500}}})
}
fn matched(snapshot: &Value, wanted: &Value) -> bool {
    if !same(&snapshot["effective"], &wanted["effective"]) {
        return false;
    }
    let (Some(actual), Some(expected)) = (
        snapshot["inspect"].as_object(),
        wanted["inspect"].as_object(),
    ) else {
        return false;
    };
    actual.len() == expected.len()
        && expected.iter().all(|(k, w)| {
            let Some(a) = actual.get(k).and_then(Value::as_object) else {
                return false;
            };
            let w = w.as_object().unwrap();
            a.len() == w.len() && w.iter().all(|(k, w)| a.get(k).is_some_and(|a| same(a, w)))
        })
}
fn uri(v: &Value, name: &str, resource: &str) -> Result<String> {
    let s = text(v, 4096)?;
    ensure!(
        !s.bytes().any(|b| b <= 0x20 || b == 0x7f) && !s.contains('\\'),
        "StableV2 URI changed"
    );
    let u = url::Url::parse(s)?;
    ensure!(
        u.scheme() == "file"
            && u.host_str().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
            && u.path().ends_with(&format!("/{name}/w/{resource}")),
        "StableV2 URI changed"
    );
    Ok(s.to_owned())
}
fn snapshot(v: &Value, id: &str) -> Result<()> {
    keys(
        v,
        &["resource", "uri", "languageId", "effective", "inspect"],
    )?;
    ensure!(
        v["resource"] == "main.txt" && v["uri"] == id && v["languageId"] == "plaintext",
        "StableV2 resource changed"
    );
    for field in ["effective", "inspect"] {
        diagnostic_value_budget(&v[field])?;
        keys(&v[field], DIAGNOSTIC_KEYS)?;
    }
    for key in DIAGNOSTIC_KEYS {
        diagnostic_setting(&v["effective"][*key], key)?;
        let m = v["inspect"][*key]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("StableV2 inspect changed"))?;
        ensure!(
            m.get("key")
                == Some(&json!(format!(
                    "{}.{}",
                    if key.starts_with("autoSave") {
                        "files"
                    } else {
                        "editor"
                    },
                    key
                ))),
            "StableV2 inspect accessor changed"
        );
        for (field, item) in m {
            match field.as_str() {
                "key" => (),
                "languageIds" => {
                    let ids = list(item, 64)?;
                    let mut seen = BTreeSet::new();
                    for id in ids {
                        let id = text(id, 128)?;
                        ensure!(
                            !id.is_empty() && seen.insert(id),
                            "StableV2 language inventory changed"
                        );
                    }
                }
                "defaultValue"
                | "globalValue"
                | "workspaceValue"
                | "workspaceFolderValue"
                | "defaultLanguageValue"
                | "globalLanguageValue"
                | "workspaceLanguageValue"
                | "workspaceFolderLanguageValue" => diagnostic_setting(item, key)?,
                _ => anyhow::bail!("StableV2 inspect field changed"),
            }
        }
    }
    Ok(())
}
fn affected(v: &Value) -> Result<()> {
    let a = list(v, 4)?;
    ensure!(!a.is_empty(), "StableV2 event scope changed");
    let mut at = 0;
    for key in a {
        let p = DIAGNOSTIC_EVENTS
            .iter()
            .position(|k| key == *k)
            .filter(|p| *p >= at)
            .ok_or_else(|| anyhow::anyhow!("StableV2 event scope changed"))?;
        at = p + 1;
    }
    Ok(())
}
// Exact first-ready semantics, preserving all possible tied cross-list orders.
// States are bounded by (128+1)^3 after list admission; no arbitrary metadata
// order is fabricated and an earlier successful quiet window cannot be skipped.
fn quiet_authorized(samples: &[Value], events: &[Value], start: u64, end: u64) -> bool {
    use std::collections::VecDeque;
    let mut pending = VecDeque::from([(0usize, 0usize, None)]);
    let mut seen = BTreeSet::new();
    while let Some((i, j, quiet)) = pending.pop_front() {
        if !seen.insert((i, j, quiet)) {
            continue;
        }
        let st = samples.get(i).and_then(|v| v["elapsedMs"].as_u64());
        let et = events.get(j).and_then(|v| v["elapsedMs"].as_u64());
        if let Some(t) = et
            && st.is_none_or(|s| t <= s)
        {
            pending.push_back((i, j + 1, None));
        }
        if let Some(t) = st
            && et.is_none_or(|e| t <= e)
        {
            let next = if samples[i]["matched"] == true {
                Some(quiet.unwrap_or(t))
            } else {
                None
            };
            if let Some(q) = next
                && t - q >= 250
            {
                if i + 1 == samples.len() && j == events.len() && q == start && t == end {
                    return true;
                }
                continue; // actual observer would already return
            }
            pending.push_back((i + 1, j, next));
        }
    }
    false
}
fn configuration(v: &Value, f: &Value, id: &str, complete: bool) -> Result<()> {
    let status = v["status"].as_str();
    if complete {
        keys(
            v,
            &[
                "protocol",
                "status",
                "name",
                "resource",
                "uri",
                "languageId",
                "expected",
                "limits",
                "samples",
                "events",
                "scope",
                "elapsedMs",
                "quietSinceMs",
                "finalSampleSequence",
            ],
        )?;
        ensure!(
            status == Some("ready"),
            "StableV2 configuration is not ready"
        );
    } else {
        optional(
            v,
            &[
                "protocol",
                "status",
                "name",
                "resource",
                "uri",
                "languageId",
                "expected",
                "limits",
                "samples",
                "events",
                "scope",
            ],
            &["error", "elapsedMs", "quietSinceMs", "finalSampleSequence"],
        )?;
        ensure!(
            matches!(status, Some("running" | "ready" | "failed")),
            "StableV2 partial configuration status changed"
        );
        if status == Some("failed") {
            error_text(&v["error"], 1024)?;
        }
    }
    compact(v, 524288)?;
    ensure!(
        v["protocol"] == "save-configuration-readiness-v1"
            && v["scope"] == READY_SCOPE
            && v["name"] == f["name"]
            && v["resource"] == "main.txt"
            && v["languageId"] == "plaintext"
            && v["uri"] == id,
        "StableV2 readiness identity changed"
    );
    ensure!(
        v["limits"]
            == json!({"deadlineMs":10000,"sampleIntervalMs":100,"quietMs":250,"maxSamples":128,"maxEvents":128,"maxValueNodes":1024,"maxValueDepth":8,"maxValueBytes":32768,"maxTotalBytes":524288}),
        "StableV2 readiness limits changed"
    );
    ensure!(
        same(&v["expected"], &expected(f)),
        "StableV2 canonical declaration changed"
    );
    let samples = list(&v["samples"], 128)?;
    let events = list(&v["events"], 128)?;
    let mut elapsed = 0;
    for (i, s) in samples.iter().enumerate() {
        keys(s, &["sequence", "elapsedMs", "snapshot", "matched"])?;
        let t = integer(&s["elapsedMs"], 9999)?;
        ensure!(
            s["sequence"] == json!(i + 1) && t >= elapsed,
            "StableV2 sample sequence changed"
        );
        elapsed = t;
        snapshot(&s["snapshot"], id)?;
        ensure!(
            s["matched"] == json!(matched(&s["snapshot"], &v["expected"])),
            "StableV2 sample match changed"
        );
    }
    elapsed = 0;
    for (i, e) in events.iter().enumerate() {
        keys(e, &["sequence", "elapsedMs", "affected", "snapshot"])?;
        let t = integer(&e["elapsedMs"], 10000)?;
        ensure!(
            e["sequence"] == json!(i + 1) && t >= elapsed,
            "StableV2 event sequence changed"
        );
        elapsed = t;
        affected(&e["affected"])?;
        snapshot(&e["snapshot"], id)?;
    }
    if complete || status == Some("ready") {
        let last = samples
            .last()
            .ok_or_else(|| anyhow::anyhow!("StableV2 ready sample absent"))?;
        let end = integer(&v["elapsedMs"], 9999)?;
        let start = integer(&v["quietSinceMs"], end)?;
        ensure!(
            v["finalSampleSequence"] == last["sequence"]
                && last["elapsedMs"] == json!(end)
                && end - start >= 250,
            "StableV2 quiet proof changed"
        );
        ensure!(
            samples
                .iter()
                .any(|s| s["elapsedMs"] == json!(start) && s["matched"] == true)
                && samples
                    .iter()
                    .filter(|s| s["elapsedMs"].as_u64().unwrap() >= start)
                    .all(|s| s["matched"] == true),
            "StableV2 quiet match changed"
        );
        ensure!(
            events
                .iter()
                .all(|e| e["elapsedMs"].as_u64().unwrap() <= start),
            "StableV2 quiet event reset ignored"
        );
        // A tied event may have run before the first quiet sample. Do not invent
        // an order between the separately recorded arrays.
        let before = samples
            .iter()
            .rev()
            .find(|s| s["elapsedMs"].as_u64().unwrap() < start);
        if let Some(previous) = before {
            ensure!(
                previous["matched"] == false
                    || events.iter().any(|e| {
                        let t = e["elapsedMs"].as_u64().unwrap();
                        t >= previous["elapsedMs"].as_u64().unwrap() && t <= start
                    }),
                "StableV2 quiet start changed"
            );
        }
        ensure!(
            events
                .iter()
                .all(|e| e["elapsedMs"].as_u64().unwrap() <= end),
            "StableV2 event exceeds final sample"
        );
        ensure!(
            quiet_authorized(samples, events, start, end),
            "StableV2 first-ready chronology changed"
        );
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct StableParticipant {
    protocol: String,
    save_reason: String,
    target_auto_save: String,
    status: String,
    resource: String,
    input: String,
    effective: AuxiliaryConfiguration,
    limits: AuxiliaryLimits,
    initial: AuxiliaryState,
    #[serde(rename = "final")]
    final_state: AuxiliaryState,
    attempts: Vec<AuxiliaryAttempt>,
    callbacks: Vec<AuxiliaryCallback>,
    scope: String,
}
fn participant(readiness: &Value, f: &Value, target_uri: &str) -> Result<()> {
    compact(readiness, 524288)?;
    auxiliary_keys(
        readiness,
        &[
            "protocol",
            "saveReason",
            "targetAutoSave",
            "status",
            "resource",
            "input",
            "effective",
            "limits",
            "initial",
            "final",
            "attempts",
            "callbacks",
            "scope",
        ],
    )?;
    auxiliary_keys(
        &readiness["effective"],
        &["formatOnSave", "formatOnSaveMode", "autoSave"],
    )?;
    auxiliary_keys(
        &readiness["limits"],
        &["maxAttempts", "deadlineMs", "retryIntervalMs"],
    )?;
    let states = &["resource", "uri", "version", "text", "dirty", "disk"];
    auxiliary_keys(&readiness["initial"], states)?;
    auxiliary_keys(&readiness["final"], states)?;
    let attempts = readiness["attempts"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid auxiliary metadata shape"))?;
    let callbacks = readiness["callbacks"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid auxiliary metadata shape"))?;
    anyhow::ensure!(
        (1..=16).contains(&attempts.len()) && callbacks.len() <= 16,
        "Auxiliary metadata exceeds bounds"
    );
    for attempt in attempts {
        auxiliary_keys(
            attempt,
            &[
                "index",
                "before",
                "prepared",
                "after",
                "callbackStart",
                "callbackCount",
                "matched",
                "elapsedMs",
            ],
        )?;
        for name in ["before", "prepared", "after"] {
            auxiliary_keys(&attempt[name], states)?;
        }
    }
    for callback in callbacks {
        auxiliary_keys(
            callback,
            &[
                "attempt",
                "uri",
                "version",
                "text",
                "resource",
                "exactDocument",
                "duringSave",
                "cancelled",
                "matched",
            ],
        )?;
    }
    let proof: StableParticipant = serde_json::from_value(readiness.clone())
        .map_err(|_| anyhow::anyhow!("Invalid auxiliary metadata shape"))?;
    anyhow::ensure!(
        proof.status == "observed"
            && proof.resource == "readiness.ini"
            && proof.input == AUX_INPUT
            && proof.scope == AUX_SCOPE
            && proof.effective.format_on_save
            && proof.effective.format_on_save_mode == "file"
            && proof.effective.auto_save == f["autosave"].as_str().unwrap()
            && proof.protocol == "save-participant-readiness-v2"
            && proof.save_reason == "explicit-auxiliary"
            && proof.target_auto_save == proof.effective.auto_save,
        "Invalid auxiliary configuration or scope"
    );
    anyhow::ensure!(
        proof.limits.max_attempts == 16
            && proof.limits.deadline_ms == 5000
            && proof.limits.retry_interval_ms == 250,
        "Auxiliary readiness limits changed"
    );
    let uri = &proof.initial.uri;
    anyhow::ensure!(
        uri.len() <= 4096
            && !uri.bytes().any(|byte| byte <= 0x20 || byte == 0x7f)
            && !uri.contains('\\'),
        "Invalid auxiliary URI"
    );
    let parsed = url::Url::parse(uri).map_err(|_| anyhow::anyhow!("Invalid auxiliary URI"))?;
    anyhow::ensure!(
        parsed.scheme() == "file"
            && parsed.host_str().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none()
            && parsed.path().starts_with('/')
            && parsed
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                == Some("readiness.ini"),
        "Invalid auxiliary URI"
    );
    ensure!(
        uri.rsplit_once("/readiness.ini")
            .map(|(parent, _)| format!("{parent}/main.txt"))
            .as_deref()
            == Some(target_uri),
        "StableV2 auxiliary workspace changed"
    );
    auxiliary_state(&proof.initial, uri)?;
    anyhow::ensure!(
        proof.initial.text == AUX_INPUT && proof.initial.disk == AUX_INPUT && !proof.initial.dirty,
        "Auxiliary initial bytes changed"
    );
    let mut previous = &proof.initial;
    let mut cursor = 0;
    let mut elapsed = 0;
    for (offset, attempt) in proof.attempts.iter().enumerate() {
        anyhow::ensure!(
            attempt.index == offset + 1 && attempt.before == *previous,
            "Auxiliary attempt continuity changed"
        );
        for state in [&attempt.before, &attempt.prepared, &attempt.after] {
            auxiliary_state(state, uri)?;
        }
        anyhow::ensure!(
            attempt.prepared.version == attempt.before.version.checked_add(1).unwrap_or(0)
                && attempt.prepared.text == format!("{}x", attempt.before.text)
                && attempt.prepared.disk == attempt.before.disk
                && attempt.prepared.dirty,
            "Auxiliary prepared edit changed"
        );
        anyhow::ensure!(
            attempt.after.version == attempt.prepared.version
                && attempt.after.text == attempt.prepared.text
                && attempt.after.disk == attempt.after.text
                && !attempt.after.dirty,
            "Auxiliary null-formatter receipt changed"
        );
        anyhow::ensure!(
            attempt.elapsed_ms >= elapsed && attempt.elapsed_ms < 5000,
            "Auxiliary readiness deadline changed"
        );
        elapsed = attempt.elapsed_ms;
        let end = attempt
            .callback_start
            .checked_add(attempt.callback_count)
            .ok_or_else(|| anyhow::anyhow!("Auxiliary callback slices changed"))?;
        anyhow::ensure!(
            attempt.callback_start >= cursor && end <= proof.callbacks.len(),
            "Auxiliary callback slices changed"
        );
        // Outside-Save callbacks are retained and checked; they cannot authorize
        // readiness and are never silently filtered from the evidence.
        for callback in &proof.callbacks[cursor..attempt.callback_start] {
            auxiliary_callback(callback, uri, None, &attempt.before)?;
        }
        let mut matched = 0;
        for callback in &proof.callbacks[attempt.callback_start..end] {
            matched += usize::from(auxiliary_callback(
                callback,
                uri,
                Some(attempt.index),
                &attempt.prepared,
            )?);
        }
        let final_attempt = offset + 1 == proof.attempts.len();
        anyhow::ensure!(
            matched == usize::from(final_attempt) && attempt.matched == final_attempt,
            "Auxiliary final positive witness changed"
        );
        cursor = end;
        previous = &attempt.after;
    }
    for callback in &proof.callbacks[cursor..] {
        auxiliary_callback(callback, uri, None, previous)?;
    }
    anyhow::ensure!(
        proof.final_state == *previous,
        "Auxiliary final snapshot changed"
    );
    Ok(())
}
fn authorization(v: &Value) -> Result<()> {
    keys(
        v,
        &["protocol", "elapsedMs", "deadlineMs", "targetAttempts"],
    )?;
    ensure!(
        v["protocol"] == "stable-save-setup-v1"
            && v["deadlineMs"] == 15000
            && v["targetAttempts"] == 0,
        "StableV2 authorization changed"
    );
    integer(&v["elapsedMs"], 14999)?;
    Ok(())
}
fn position(v: &Value, t: &str) -> Result<()> {
    let a = list(v, 2)?;
    ensure!(a.len() == 2, "StableV2 position changed");
    let line = integer(&a[0], i32::MAX as u64)? as usize;
    let col = integer(&a[1], i32::MAX as u64)? as usize;
    let row = t
        .split('\n')
        .nth(line)
        .ok_or_else(|| anyhow::anyhow!("StableV2 position line changed"))?;
    let row = row.strip_suffix('\r').unwrap_or(row);
    let mut units = 0;
    let mut valid = col == 0;
    for c in row.chars() {
        units += c.len_utf16();
        valid |= units == col;
    }
    ensure!(valid, "StableV2 position surrogate or EOL changed");
    Ok(())
}
fn selections(v: &Value, t: &str) -> Result<()> {
    let a = list(v, 256)?;
    ensure!(!a.is_empty(), "StableV2 selections empty");
    for s in a {
        let p = list(s, 4)?;
        ensure!(p.len() == 4, "StableV2 selection shape changed");
        position(&json!([p[0], p[1]]), t)?;
        position(&json!([p[2], p[3]]), t)?;
    }
    Ok(())
}
fn model(v: &Value) -> Result<()> {
    keys(v, &["text", "version", "dirty", "selections", "disk"])?;
    let t = text(&v["text"], 65536)?;
    text(&v["disk"], 65536)?;
    ensure!(
        integer(&v["version"], i32::MAX as u64)? > 0,
        "StableV2 version changed"
    );
    boolean(&v["dirty"])?;
    selections(&v["selections"], t)
}
fn observed(v: &Value) -> Result<()> {
    keys(
        v,
        &["action", "resource", "text", "dirty", "primary", "disk"],
    )?;
    ensure!(
        v["resource"] == "main.txt",
        "StableV2 observed resource changed"
    );
    text(&v["action"], 128)?;
    let t = text(&v["text"], 65536)?;
    text(&v["disk"], 65536)?;
    boolean(&v["dirty"])?;
    keys(&v["primary"], &["anchor", "cursor"])?;
    for k in ["anchor", "cursor"] {
        integer(&v["primary"][k], t.chars().count() as u64)?;
    }
    Ok(())
}
fn definitions() -> Value {
    json!([{"title":"Synthetic imports","kind":"source.organizeImports"},{"title":"Synthetic child fix","kind":"source.fixAll.child"},
        {"title":"Synthetic root fix","kind":"source.fixAll"},{"title":"Synthetic unrelated prefix","kind":"source.fixAllX"}])
}
fn phase_version(v: &Value, previous: &mut u64) -> Result<()> {
    ensure!(
        matches!(v["phase"].as_str(), Some("setup" | "target")),
        "StableV2 event phase changed"
    );
    let n = integer(&v["version"], i32::MAX as u64)?;
    ensure!(n > 0 && n >= *previous, "StableV2 event version changed");
    *previous = n;
    text(&v["text"], 65536)?;
    boolean(&v["dirty"])?;
    Ok(())
}
fn event_lists(raw: &Value) -> Result<()> {
    let mut n = 0;
    for c in list(&raw["callbacks"], 256)? {
        keys(
            c,
            &[
                "phase",
                "only",
                "triggerKind",
                "version",
                "text",
                "cancelled",
                "range",
                "returned",
            ],
        )?;
        ensure!(
            matches!(c["phase"].as_str(), Some("setup" | "target")),
            "StableV2 callback phase changed"
        );
        let t = text(&c["text"], 65536)?;
        let version = integer(&c["version"], i32::MAX as u64)?;
        ensure!(
            version > 0 && version >= n,
            "StableV2 callback version changed"
        );
        n = version;
        if !c["only"].is_null() {
            text(&c["only"], 128)?;
        }
        ensure!(
            matches!(c["triggerKind"].as_u64(), Some(1 | 2)),
            "StableV2 trigger changed"
        );
        boolean(&c["cancelled"])?;
        let r = list(&c["range"], 4)?;
        ensure!(r.len() == 4, "StableV2 range changed");
        position(&json!([r[0], r[1]]), t)?;
        position(&json!([r[2], r[3]]), t)?;
        ensure!(
            (r[0].as_u64().unwrap(), r[1].as_u64().unwrap())
                <= (r[2].as_u64().unwrap(), r[3].as_u64().unwrap()),
            "StableV2 reversed range"
        );
        ensure!(
            same(&c["returned"], &definitions()),
            "StableV2 provider inventory changed"
        );
    }
    n = 0;
    for s in list(&raw["saved"], 256)? {
        keys(s, &["phase", "version", "text", "dirty", "disk"])?;
        phase_version(s, &mut n)?;
        text(&s["disk"], 65536)?;
        ensure!(
            s["text"] == s["disk"] && s["dirty"] == false,
            "StableV2 Save receipt changed"
        );
    }
    n = 0;
    for e in list(&raw["changes"], 256)? {
        keys(
            e,
            &["phase", "version", "text", "dirty", "reason", "changes"],
        )?;
        phase_version(e, &mut n)?;
        ensure!(
            matches!(e["reason"].as_u64(), Some(1 | 2)) || e["reason"].is_null(),
            "StableV2 change reason changed"
        );
        // rangeOffset/rangeLength refer to the prior model, not this post-event
        // text. Preserve them without validating against the wrong revision.
        for c in list(&e["changes"], 256)? {
            keys(c, &["rangeOffset", "rangeLength", "text"])?;
            integer(&c["rangeOffset"], i32::MAX as u64)?;
            integer(&c["rangeLength"], i32::MAX as u64)?;
            text(&c["text"], 65536)?;
        }
    }
    Ok(())
}
fn actions_sequence(f: &Value) -> Vec<&'static str> {
    if f["autosave"] == "afterDelay" {
        vec!["type", "files.autoSave.afterDelay"]
    } else {
        let mut s = vec!["type", "workbench.action.files.save"];
        s.extend(["undo"; 5]);
        s.extend(["redo"; 5]);
        s
    }
}
fn step(v: &Value, action: &str, saved: &[Value]) -> Result<()> {
    if action == "workbench.action.files.save" || action == "files.autoSave.afterDelay" {
        keys(v, &["action", "acknowledgement", "settlement"])?;
        keys(&v["acknowledgement"], &["elapsedMs", "event"])?;
        integer(&v["acknowledgement"]["elapsedMs"], 300000)?;
        ensure!(
            saved.len() == 1 && v["acknowledgement"]["event"] == saved[0],
            "StableV2 acknowledgement changed"
        );
    } else {
        keys(v, &["action", "settlement"])?;
    }
    ensure!(v["action"] == action, "StableV2 target action changed");
    keys(&v["settlement"], &["elapsedMs", "scope"])?;
    ensure!(
        integer(&v["settlement"]["elapsedMs"], 300000)? >= 100
            && v["settlement"]["scope"] == STEP_SCOPE,
        "StableV2 settlement changed"
    );
    Ok(())
}
fn success(raw: &Value, f: &Value) -> Result<()> {
    keys(
        raw,
        &[
            "name",
            "setup",
            "observations",
            "steps",
            "callbacks",
            "saved",
            "changes",
            "stableConfiguration",
            "targetCommandsIssued",
            "configurationDiagnostics",
            "commandInventory",
            "executeApiListed",
            "scope",
        ],
    )?;
    ensure!(
        raw["name"] == f["name"] && raw["scope"] == RAW_SCOPE,
        "StableV2 raw identity changed"
    );
    ensure!(
        raw["commandInventory"] == json!(["redo", "type", "undo", "workbench.action.files.save"]),
        "StableV2 command inventory changed"
    );
    boolean(&raw["executeApiListed"])?;
    let setup = &raw["setup"];
    keys(
        setup,
        &[
            "saveParticipantReadiness",
            "configurationReadiness",
            "configurationCheckpoint",
            "setupAuthorization",
            "effective",
            "effectiveKeys",
            "configurationInspect",
            "before",
            "after",
            "warmKinds",
            "scope",
        ],
    )?;
    let id = uri(
        &setup["configurationReadiness"]["uri"],
        f["name"].as_str().unwrap(),
        "main.txt",
    )?;
    configuration(&setup["configurationReadiness"], f, &id, true)?;
    participant(&setup["saveParticipantReadiness"], f, &id)?;
    authorization(&setup["setupAuthorization"])?;
    snapshot(&setup["configurationCheckpoint"], &id)?;
    let e = expected(f);
    ensure!(
        matched(&setup["configurationCheckpoint"], &e)
            && same(&setup["effective"], &e["effective"]),
        "StableV2 setup configuration changed"
    );
    let expected_keys = actions(f)
        .as_object()
        .map(|m| json!(m.keys().collect::<Vec<_>>()))
        .unwrap_or(Value::Null);
    ensure!(
        setup["effectiveKeys"] == expected_keys
            && setup["configurationInspect"] == e["inspect"]["codeActionsOnSave"],
        "StableV2 setup inspection changed"
    );
    model(&setup["before"])?;
    model(&setup["after"])?;
    ensure!(
        setup["before"] == setup["after"]
            && setup["before"]["text"] == TEXT
            && setup["before"]["disk"] == TEXT
            && setup["before"]["dirty"] == false,
        "StableV2 unchanged warm proof changed"
    );
    ensure!(
        setup["before"]["selections"] == json!([[5, 0, 5, 0]]),
        "StableV2 initial setup selection changed"
    );
    ensure!(
        setup["warmKinds"]
            == json!([
                "source.organizeImports",
                "source.fixAll.child",
                "source.fixAll",
                "source.fixAllX"
            ])
            && setup["scope"] == SETUP_SCOPE,
        "StableV2 warm inventory changed"
    );
    event_lists(raw)?;
    let callbacks = list(&raw["callbacks"], 256)?;
    ensure!(
        callbacks.first().is_some_and(|c| c["phase"] == "setup"
            && c["only"] == "source"
            && c["triggerKind"] == 1
            && c["text"] == TEXT
            && c["version"] == setup["before"]["version"])
            && callbacks.iter().skip(1).all(|c| c["phase"] == "target"),
        "StableV2 warm callback changed"
    );
    let sequence = actions_sequence(f);
    let obs = list(&raw["observations"], 256)?;
    let steps = list(&raw["steps"], 256)?;
    ensure!(
        obs.len() == sequence.len() + 1
            && steps.len() == sequence.len()
            && raw["targetCommandsIssued"]
                == json!(if f["autosave"] == "afterDelay" { 1 } else { 12 }),
        "StableV2 target count changed"
    );
    observed(&obs[0])?;
    ensure!(
        obs[0]["action"] == "initial"
            && obs[0]["text"] == setup["before"]["text"]
            && obs[0]["disk"] == setup["before"]["disk"]
            && obs[0]["dirty"] == setup["before"]["dirty"],
        "StableV2 initial observation changed"
    );
    let start = TEXT.chars().count();
    ensure!(
        obs[0]["primary"] == json!({"anchor":start,"cursor":start}),
        "StableV2 initial caret changed"
    );
    let saved = list(&raw["saved"], 256)?;
    for ((action, o), s) in sequence.iter().zip(&obs[1..]).zip(steps) {
        observed(o)?;
        ensure!(o["action"] == *action, "StableV2 observation order changed");
        step(s, action, saved)?;
    }
    validate_configuration_diagnostics(raw)?;
    let stable = &raw["stableConfiguration"];
    keys(
        stable,
        &["valid", "targetChecks", "finalCheckpoint", "scope"],
    )?;
    ensure!(
        stable["scope"] == STABLE_SCOPE,
        "StableV2 target scope changed"
    );
    snapshot(&stable["finalCheckpoint"], &id)?;
    let records = list(&raw["configurationDiagnostics"]["records"], 128)?;
    let selected: Vec<_> = records.iter().filter(|r| r["phase"] == "target").collect();
    let checks = list(&stable["targetChecks"], 128)?;
    ensure!(
        checks.len() == selected.len(),
        "StableV2 target check inventory changed"
    );
    let mut valid = matched(&stable["finalCheckpoint"], &e);
    for (c, r) in checks.iter().zip(selected) {
        keys(c, &["sequence", "origin", "matched"])?;
        let actual = matched(&r["snapshot"], &e);
        ensure!(
            c["sequence"] == r["sequence"]
                && c["origin"] == r["origin"]
                && c["matched"] == json!(actual),
            "StableV2 target check correlation changed"
        );
        valid &= actual;
    }
    ensure!(
        stable["valid"] == json!(valid),
        "StableV2 target valid flag changed"
    );
    // Retain mismatching evidence, then refuse successful qualification. Never
    // replace it with an older platform projection or omit a case.
    ensure!(valid, "StableV2 target configuration was not stable");
    Ok(())
}
fn launch(f: &Value) -> Value {
    let mut user = json!({"telemetry.telemetryLevel":"off","update.mode":"none","extensions.autoUpdate":false,
        "extensions.autoCheckUpdates":false,"security.workspace.trust.enabled":false,"workbench.startupEditor":"none",
        "workbench.editor.enablePreview":false,"editor.parameterHints.enabled":false,"editor.quickSuggestions":false,
        "editor.formatOnSave":false,"editor.formatOnPaste":false,"editor.formatOnType":false,
        "[ini]":{"editor.formatOnSave":true,"editor.formatOnSaveMode":"file"},"editor.codeActions.triggerOnFocusChange":false,
        "editor.codeActionsOnSave":{},"editor.detectIndentation":false,"editor.tabSize":2,"editor.insertSpaces":true,
        "files.autoSave":f["autosave"],"files.autoSaveDelay":500,"files.trimTrailingWhitespace":false,
        "files.insertFinalNewline":false,"files.trimFinalNewlines":false});
    for (k, v) in f["user"].as_object().unwrap() {
        user[k] = v.clone();
    }
    json!({"name":f["name"],"profileSettings":serde_json::to_string(&user).unwrap(),
        "workspaceSettings":serde_json::to_string(&f["workspace"]).unwrap(),"targetText":TEXT,"auxiliaryText":AUX_INPUT})
}
fn launcher(v: &Value) -> Result<()> {
    keys(v, &["path", "bytes", "sha256"])?;
    let p = text(&v["path"], 4096)?;
    ensure!(
        !p.is_empty()
            && (p.starts_with('/')
                || (p.len() >= 3
                    && p.as_bytes()[0].is_ascii_alphabetic()
                    && p.as_bytes()[1] == b':'
                    && matches!(p.as_bytes()[2], b'/' | b'\\'))),
        "StableV2 launcher path changed"
    );
    ensure!(
        integer(&v["bytes"], 512 * MIB as u64)? > 0,
        "StableV2 launcher size changed"
    );
    digest(&v["sha256"])
}
fn provenance(
    v: &Value,
    manifest: &Value,
    platform: &Value,
    arch: &Value,
    f: &Value,
    failed: bool,
) -> Result<()> {
    keys(
        v,
        &[
            "version",
            "commit",
            "platform",
            "architecture",
            "observedAt",
            "name",
            "productSha256",
            "launcher",
            if failed {
                "failureSha256"
            } else {
                "evidenceSha256"
            },
            "sources",
            "profileSettingsSha256",
            "workspaceSettingsSha256",
            "scope",
        ],
    )?;
    ensure!(
        v["version"] == "1.95.0"
            && v["commit"] == PIN
            && v["platform"] == *platform
            && v["architecture"] == *arch
            && v["name"] == f["name"],
        "StableV2 product identity changed"
    );
    let stamp = text(&v["observedAt"], 128)?;
    ensure!(
        stamp.len() == 24
            && stamp.bytes().enumerate().all(|(i, b)| match i {
                4 | 7 => b == b'-',
                10 => b == b'T',
                13 | 16 => b == b':',
                19 => b == b'.',
                23 => b == b'Z',
                _ => b.is_ascii_digit(),
            }),
        "StableV2 observed timestamp changed"
    );
    digest(&v["productSha256"])?;
    digest(&v["profileSettingsSha256"])?;
    digest(&v["workspaceSettingsSha256"])?;
    launcher(&v["launcher"])?;
    ensure!(
        v["sources"] == *manifest,
        "StableV2 per-case sources changed"
    );
    digest(
        &v[if failed {
            "failureSha256"
        } else {
            "evidenceSha256"
        }],
    )?;
    ensure!(
        v["scope"]
            == if failed {
                FAILURE_PROVENANCE_SCOPE
            } else {
                RAW_SCOPE
            },
        "StableV2 provenance scope changed"
    );
    Ok(())
}
#[path = "stable_v2/current_capture.rs"]
mod current_capture;

/// Load fully admitted current rows, or the exact archived Linux cohort locally.
/// A current artifact requires the independently supplied revision environment pair.
pub(super) fn current_corpus() -> Result<Vec<Value>> {
    current_capture::current_corpus()
}

struct Artifact {
    root: PathBuf,
    cases: Vec<Value>,
    trace: Vec<Value>,
    raw: Vec<Value>,
    proof: Value,
    manifest: Value,
}
impl Artifact {
    fn admit(root: &Path) -> Result<Self> {
        Self::admit_with_context(root, current_capture::Context::Archived133)
    }

    fn admit_with_context(root: &Path, context: current_capture::Context<'_>) -> Result<Self> {
        // Validate trusted caller input before any artifact I/O. The envelope,
        // source hashes and metadata guards below are shared without exceptions.
        let expected_revision = context.expected_revision()?;
        let input = root.join(INPUT);
        let output = root.join(OUTPUT);
        let result = output.join("result");
        ensure!(
            inventory(root, 2)?
                == ["tests", "target"]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            "StableV2 artifact root inventory changed"
        );
        ensure!(
            inventory(&root.join("tests"), 1)?
                == ["vscode-reference-save-actions-stable-configuration-candidate"]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            "StableV2 input directory changed"
        );
        ensure!(
            inventory(&root.join("target"), 1)?
                == ["save-actions-stable-configuration"]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            "StableV2 output directory changed"
        );

        ensure!(
            std::str::from_utf8(&bounded(&output.join("candidate-revision.txt"), 128)?)?.trim()
                == expected_revision,
            "StableV2 source revision changed"
        );
        let manifest = read(&input.join("source-sha256.json"), 16384)?;
        keys(
            &manifest,
            &SOURCES.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        )?;
        let mut source_names: BTreeSet<String> =
            SOURCES.iter().map(|(n, _)| (*n).to_owned()).collect();
        for name in [
            "source-sha256.json",
            "README.md",
            "save-code-actions-readiness.test.cjs",
            "save-code-actions-diagnostics.test.cjs",
            "save-configuration-ready.test.cjs",
        ] {
            source_names.insert(name.to_owned());
            bounded(
                &input.join(name),
                if name == "source-sha256.json" {
                    16384
                } else {
                    MIB
                },
            )?;
        }
        let actual_sources = inventory(&input, 15)?;
        ensure!(
            actual_sources == source_names,
            "StableV2 supplemental source inventory changed"
        );
        let actual_output = inventory(&output, 3)?;
        ensure!(
            actual_output
                == ["candidate-revision.txt", "capture.log", "result"]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            "StableV2 output inventory changed"
        );
        // Raw capture log is retained, bounded and never parsed/normalized.
        bounded(&output.join("capture.log"), 64 * MIB)?;
        let mut total = 0;
        for (name, digest) in SOURCES {
            ensure!(manifest[*name] == *digest, "StableV2 frozen source changed");
            let bytes = bounded(&input.join(name), MIB.min(4 * MIB - total))?;
            total += bytes.len();
            ensure!(sha(&bytes) == *digest, "StableV2 source bytes changed");
        }
        let cases = list(&read(&input.join("save-code-actions-cases.json"), MIB)?, 14)?.to_vec();
        ensure!(cases.len() == 14, "StableV2 case count changed");
        let mut names = BTreeSet::new();
        let mut files: BTreeSet<String> = [
            "save-code-actions.json",
            "save-code-actions-evidence.json",
            "save-code-actions-provenance.json",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        for f in &cases {
            keys(f, &["name", "autosave", "user", "workspace"])?;
            let name = text(&f["name"], 128)?;
            ensure!(
                !name.is_empty()
                    && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
                    && names.insert(name),
                "StableV2 case identity changed"
            );
            for suffix in [
                "evidence",
                "provenance",
                "launch-inputs",
                "runtime-settings",
                "worker-io-provenance",
            ] {
                files.insert(format!("{name}-{suffix}.json"));
            }
        }
        let actual = inventory(&result, 74)?;
        ensure!(actual == files, "StableV2 result inventory changed");
        let trace_bytes = bounded(&result.join("save-code-actions.json"), 64 * MIB)?;
        let raw_bytes = bounded(&result.join("save-code-actions-evidence.json"), 64 * MIB)?;
        let proof = read(&result.join("save-code-actions-provenance.json"), MIB)?;
        keys(
            &proof,
            &[
                "version",
                "commit",
                "platform",
                "architecture",
                "caseCount",
                "snapshotCount",
                "traceSha256",
                "evidenceSha256",
                "casesSha256",
                "fixtureReceipts",
                "runs",
            ],
        )?;
        ensure!(
            proof["version"] == "1.95.0"
                && proof["commit"] == PIN
                && proof["caseCount"] == 14
                && proof["snapshotCount"] == 162,
            "StableV2 aggregate identity changed"
        );
        ensure!(
            matches!(
                proof["platform"].as_str(),
                Some("linux" | "darwin" | "win32")
            ) && matches!(proof["architecture"].as_str(), Some("x64" | "arm64")),
            "StableV2 platform changed"
        );
        ensure!(
            proof["traceSha256"] == sha(&trace_bytes)
                && proof["evidenceSha256"] == sha(&raw_bytes)
                && proof["casesSha256"] == manifest["save-code-actions-cases.json"],
            "StableV2 aggregate digest changed"
        );
        let trace = list(&parse(&trace_bytes)?, 14)?.to_vec();
        let raw = list(&parse(&raw_bytes)?, 14)?.to_vec();
        let runs = list(&proof["runs"], 14)?;
        let receipts = list(&proof["fixtureReceipts"], 14)?;
        ensure!(
            trace.len() == 14 && raw.len() == 14 && runs.len() == 14 && receipts.len() == 14,
            "StableV2 complete case inventory changed"
        );
        let mut frames = 0;
        for (i, f) in cases.iter().enumerate() {
            let name = f["name"].as_str().unwrap();
            let row = &trace[i];
            let raw = &raw[i];
            let run = &runs[i];
            let receipt = &receipts[i];
            let bytes = bounded(&result.join(format!("{name}-evidence.json")), 2 * MIB)?;
            ensure!(
                parse(&bytes)? == *raw && run["evidenceSha256"] == sha(&bytes),
                "StableV2 individual evidence changed"
            );
            ensure!(
                read(&result.join(format!("{name}-provenance.json")), 65536)? == *run,
                "StableV2 individual provenance changed"
            );
            provenance(
                run,
                &manifest,
                &proof["platform"],
                &proof["architecture"],
                f,
                false,
            )?;
            ensure!(
                run["launcher"] == runs[0]["launcher"]
                    && run["productSha256"] == runs[0]["productSha256"],
                "StableV2 product continuity changed"
            );
            let launch_bytes = bounded(&result.join(format!("{name}-launch-inputs.json")), 65536)?;
            let launch_raw = parse(&launch_bytes)?;
            ensure!(
                same(&launch_raw, &launch(f)),
                "StableV2 launch bytes contract changed"
            );
            ensure!(
                run["profileSettingsSha256"]
                    == sha(launch_raw["profileSettings"].as_str().unwrap().as_bytes())
                    && run["workspaceSettingsSha256"]
                        == sha(launch_raw["workspaceSettings"].as_str().unwrap().as_bytes()),
                "StableV2 launch settings digest changed"
            );
            let runtime_bytes =
                bounded(&result.join(format!("{name}-runtime-settings.json")), 65536)?;
            let runtime = parse(&runtime_bytes)?;
            keys(&runtime, &["name", "profileSettings", "workspaceSettings"])?;
            ensure!(
                runtime["name"] == f["name"],
                "StableV2 runtime receipt identity changed"
            );
            text(&runtime["profileSettings"], 60000)?;
            text(&runtime["workspaceSettings"], 60000)?;
            ensure!(
                read(
                    &result.join(format!("{name}-worker-io-provenance.json")),
                    65536
                )? == *receipt,
                "StableV2 IO receipt changed"
            );
            keys(
                receipt,
                &[
                    "name",
                    "casePassed",
                    "launcher",
                    "launchInputsSha256",
                    "runtimeSettingsSha256",
                    "scope",
                ],
            )?;
            ensure!(
                receipt["name"] == f["name"]
                    && receipt["casePassed"] == true
                    && receipt["launcher"] == run["launcher"]
                    && receipt["scope"] == IO_SCOPE
                    && receipt["launchInputsSha256"] == sha(&launch_bytes)
                    && receipt["runtimeSettingsSha256"] == sha(&runtime_bytes),
                "StableV2 IO digests changed"
            );
            keys(
                row,
                &[
                    "name",
                    "effective",
                    "effectiveKeys",
                    "observations",
                    "callbacks",
                ],
            )?;
            let projected:Vec<_>=list(&raw["callbacks"],256)?.iter().filter(|c|c["phase"]=="target").map(|c|json!({"only":c["only"],"triggerKind":c["triggerKind"],"text":c["text"],"returned":c["returned"]})).collect();
            ensure!(
                row["name"] == f["name"]
                    && row["effective"] == raw["setup"]["effective"]
                    && row["effectiveKeys"] == raw["setup"]["effectiveKeys"]
                    && row["observations"] == raw["observations"]
                    && row["callbacks"] == json!(projected),
                "StableV2 raw projection changed"
            );
            frames += list(&row["observations"], 256)?.len();
        }
        ensure!(frames == 162, "StableV2 frame count changed");
        // Only after full source/receipt/raw admission does metadata authorize.
        for (raw, f) in raw.iter().zip(&cases) {
            success(raw, f)?;
        }
        Ok(Self {
            root: root.to_owned(),
            cases,
            trace,
            raw,
            proof,
            manifest,
        })
    }
}
fn partial_participant(v: &Value, f: &Value, id: &str) -> Result<()> {
    compact(v, 524288)?;
    optional(
        v,
        &[
            "protocol",
            "status",
            "resource",
            "input",
            "saveReason",
            "targetAutoSave",
            "limits",
            "attempts",
            "callbacks",
        ],
        &[
            "effective",
            "initial",
            "final",
            "pendingAttempt",
            "error",
            "scope",
        ],
    )?;
    ensure!(
        v["protocol"] == "save-participant-readiness-v2"
            && v["resource"] == "readiness.ini"
            && v["input"] == AUX_INPUT
            && v["saveReason"] == "explicit-auxiliary"
            && v["targetAutoSave"] == f["autosave"],
        "StableV2 partial auxiliary identity changed"
    );
    ensure!(
        v["limits"] == json!({"maxAttempts":16,"deadlineMs":5000,"retryIntervalMs":250}),
        "StableV2 partial auxiliary limits changed"
    );
    let status = v["status"].as_str();
    ensure!(
        matches!(status, Some("starting" | "running" | "observed" | "failed")),
        "StableV2 partial auxiliary status changed"
    );
    if status == Some("observed") {
        return participant(v, f, id);
    }
    if status == Some("failed") {
        error_text(&v["error"], 1024)?;
    } else {
        ensure!(
            v.get("error").is_none(),
            "StableV2 unexpected auxiliary error"
        );
    }
    let attempts = list(&v["attempts"], 16)?;
    let callbacks = list(&v["callbacks"], 16)?;
    let Some(initial) = v.get("initial") else {
        ensure!(
            v.get("effective").is_none()
                && v.get("pendingAttempt").is_none()
                && attempts.is_empty()
                && callbacks.is_empty(),
            "StableV2 partial auxiliary initialization changed"
        );
        return Ok(());
    };
    keys(
        &v["effective"],
        &["formatOnSave", "formatOnSaveMode", "autoSave"],
    )?;
    ensure!(
        v["effective"]
            == json!({"formatOnSave":true,"formatOnSaveMode":"file","autoSave":f["autosave"]}),
        "StableV2 partial auxiliary configuration changed"
    );
    let initial: AuxiliaryState = serde_json::from_value(initial.clone())?;
    ensure!(
        initial
            .uri
            .rsplit_once("/readiness.ini")
            .map(|(p, _)| format!("{p}/main.txt"))
            .as_deref()
            == Some(id),
        "StableV2 partial auxiliary workspace changed"
    );
    auxiliary_state(&initial, &initial.uri)?;
    ensure!(
        initial.text == AUX_INPUT && initial.disk == AUX_INPUT && !initial.dirty,
        "StableV2 partial auxiliary initial changed"
    );
    let mut previous = initial;
    let mut cursor = 0;
    let mut elapsed = 0;
    for c in callbacks {
        keys(
            c,
            &[
                "attempt",
                "uri",
                "version",
                "text",
                "resource",
                "exactDocument",
                "duringSave",
                "cancelled",
                "matched",
            ],
        )?;
    }
    let callbacks: Vec<AuxiliaryCallback> = callbacks
        .iter()
        .map(|c| serde_json::from_value(c.clone()))
        .collect::<std::result::Result<_, _>>()?;
    for (i, a) in attempts.iter().enumerate() {
        let a: AuxiliaryAttempt = serde_json::from_value(a.clone())?;
        ensure!(
            a.index == i + 1
                && a.before == previous
                && !a.matched
                && a.elapsed_ms >= elapsed
                && a.elapsed_ms <= 300000,
            "StableV2 partial auxiliary attempt changed"
        );
        elapsed = a.elapsed_ms;
        for s in [&a.before, &a.prepared, &a.after] {
            auxiliary_state(s, &previous.uri)?;
        }
        ensure!(
            a.prepared.text == format!("{}x", previous.text)
                && a.prepared.version == previous.version + 1
                && a.prepared.disk == previous.disk
                && a.prepared.dirty
                && a.after.text == a.prepared.text
                && a.after.version == a.prepared.version
                && a.after.disk == a.after.text
                && !a.after.dirty,
            "StableV2 partial auxiliary transaction changed"
        );
        let end = a
            .callback_start
            .checked_add(a.callback_count)
            .filter(|n| *n <= callbacks.len())
            .ok_or_else(|| anyhow::anyhow!("StableV2 partial auxiliary slices changed"))?;
        ensure!(
            a.callback_start >= cursor,
            "StableV2 partial auxiliary slices changed"
        );
        for c in &callbacks[cursor..a.callback_start] {
            auxiliary_callback(c, &previous.uri, None, &previous)?;
        }
        for c in &callbacks[a.callback_start..end] {
            ensure!(
                !auxiliary_callback(c, &previous.uri, Some(a.index), &a.prepared)?,
                "StableV2 partial auxiliary undisclosed positive witness"
            );
        }
        cursor = end;
        previous = a.after;
    }
    if let Some(p) = v.get("pendingAttempt") {
        keys(p, &["index", "before", "prepared", "callbackStart"])?;
        let before: AuxiliaryState = serde_json::from_value(p["before"].clone())?;
        let prepared: AuxiliaryState = serde_json::from_value(p["prepared"].clone())?;
        ensure!(
            p["index"] == json!(attempts.len() + 1)
                && before == previous
                && prepared.text == format!("{}x", before.text)
                && prepared.version == before.version + 1
                && prepared.disk == before.disk
                && prepared.dirty,
            "StableV2 pending auxiliary changed"
        );
        auxiliary_state(&prepared, &previous.uri)?;
        let start = integer(&p["callbackStart"], callbacks.len() as u64)? as usize;
        ensure!(start >= cursor, "StableV2 pending auxiliary slices changed");
        for c in &callbacks[cursor..start] {
            auxiliary_callback(c, &previous.uri, None, &previous)?;
        }
        for c in &callbacks[start..] {
            auxiliary_callback(c, &previous.uri, Some(attempts.len() + 1), &prepared)?;
        }
    } else {
        for c in &callbacks[cursor..] {
            auxiliary_callback(c, &previous.uri, None, &previous)?;
        }
    }
    ensure!(
        v.get("final").is_none() && v.get("scope").is_none(),
        "StableV2 partial auxiliary final changed"
    );
    Ok(())
}
fn partial_diagnostics(v: &Value, prefix: &Value, id: &str, phase: &str) -> Result<()> {
    keys(
        v,
        &[
            "protocol", "limits", "initial", "final", "records", "status", "error", "scope",
        ],
    )?;
    ensure!(
        v["protocol"] == "save-configuration-diagnostics-v1"
            && v["status"] == "partial-failure"
            && v["scope"] == PARTIAL_SCOPE
            && v["limits"]
                == json!({"maxRecords":128,"maxValueNodes":1024,"maxValueDepth":8,"maxValueBytes":32768,"maxTotalBytes":524288}),
        "StableV2 partial diagnostics protocol changed"
    );
    compact(v, 524288)?;
    if !v["error"].is_null() {
        error_text(&v["error"], 1024)?;
    }
    ensure!(
        diagnostic_snapshot(&v["initial"], id)?.is_none(),
        "StableV2 partial diagnostic initial changed"
    );
    let records = list(&v["records"], 128)?;
    ensure!(
        &v["final"]
            == records
                .last()
                .map(|r| &r["snapshot"])
                .unwrap_or(&v["initial"]),
        "StableV2 partial diagnostic final changed"
    );
    let mut t = 0;
    let mut version = None;
    let mut target = false;
    let mut provider = 0;
    let mut save = 0;
    let callbacks = list(&prefix["callbacks"], 256)?;
    let saved = list(&prefix["saved"], 256)?;
    for (i, r) in records.iter().enumerate() {
        keys(
            r,
            &[
                "sequence",
                "origin",
                "phase",
                "elapsedMs",
                "snapshot",
                "context",
            ],
        )?;
        let now = integer(&r["elapsedMs"], 300000)?;
        ensure!(
            r["sequence"] == json!(i + 1)
                && now >= t
                && matches!(r["phase"].as_str(), Some("setup" | "target"))
                && !(target && r["phase"] == "setup")
                && !(phase == "setup" && r["phase"] == "target"),
            "StableV2 partial diagnostic sequence changed"
        );
        target |= r["phase"] == "target";
        t = now;
        let next = diagnostic_snapshot(&r["snapshot"], id)?;
        ensure!(
            version.is_none() || next.is_some_and(|n| n >= version.unwrap()),
            "StableV2 partial diagnostic lifetime changed"
        );
        version = next;
        match r["origin"].as_str() {
            Some("configuration-change") => {
                keys(&r["context"], &["affected"])?;
                affected(&r["context"]["affected"])?;
            }
            Some("source-provider") => {
                keys(&r["context"], &["only", "triggerKind", "cancelled"])?;
                let c = callbacks.get(provider);
                provider += 1;
                if let Some(c) = c {
                    ensure!(
                        r["phase"] == c["phase"]
                            && r["snapshot"]["version"] == c["version"]
                            && r["context"]["only"] == c["only"]
                            && r["context"]["triggerKind"] == c["triggerKind"]
                            && r["context"]["cancelled"] == c["cancelled"],
                        "StableV2 partial callback correlation changed"
                    );
                } else {
                    ensure!(
                        provider == callbacks.len() + 1
                            && prefix.get("recordingError").is_some()
                            && i + 1 == records.len(),
                        "StableV2 partial callback missing"
                    );
                }
            }
            Some("did-save") => {
                ensure!(
                    r["context"].is_null(),
                    "StableV2 partial save context changed"
                );
                if let Some(s) = saved.get(save) {
                    ensure!(
                        r["phase"] == s["phase"] && r["snapshot"]["version"] == s["version"],
                        "StableV2 partial save correlation changed"
                    );
                } else {
                    ensure!(
                        save == saved.len()
                            && prefix.get("recordingError").is_some()
                            && i + 1 == records.len(),
                        "StableV2 partial save missing"
                    );
                }
                save += 1;
            }
            _ => anyhow::bail!("StableV2 partial diagnostic origin changed"),
        }
    }
    ensure!(
        provider >= callbacks.len() && (save >= saved.len() || !v["error"].is_null()),
        "StableV2 partial diagnostic prefix missing"
    );
    Ok(())
}
fn current(v: &Value, id: &str) -> Result<()> {
    compact(v, 163840)?;
    if v["status"] == "observed" {
        keys(
            v,
            &[
                "status",
                "resource",
                "uri",
                "languageId",
                "version",
                "text",
                "dirty",
                "selections",
                "disk",
                "diskError",
            ],
        )?;
        ensure!(
            v["resource"] == "main.txt" && v["uri"] == id && v["languageId"] == "plaintext",
            "StableV2 failure current resource changed"
        );
        ensure!(
            integer(&v["version"], i32::MAX as u64)? > 0,
            "StableV2 failure current version changed"
        );
        let t = text(&v["text"], 65536)?;
        boolean(&v["dirty"])?;
        selections(&v["selections"], t)?;
    } else {
        keys(v, &["status", "error", "disk", "diskError"])?;
        ensure!(
            v["status"] == "unavailable",
            "StableV2 failure current status changed"
        );
        error_text(&v["error"], 1024)?;
    }
    if v["disk"].is_null() {
        ensure!(
            !v["diskError"].is_null(),
            "StableV2 failure disk error missing"
        );
    } else {
        text(&v["disk"], 65536)?;
        ensure!(
            v["diskError"].is_null(),
            "StableV2 failure disk error changed"
        );
    }
    if !v["diskError"].is_null() {
        error_text(&v["diskError"], 1024)?;
    }
    Ok(())
}
fn failure(v: &Value, f: &Value) -> Result<()> {
    compact(v, 2 * MIB)?;
    if v["phase"] == "before-observer" {
        keys(
            v,
            &[
                "protocol",
                "name",
                "phase",
                "targetStarted",
                "error",
                "scope",
            ],
        )?;
        ensure!(
            v["protocol"] == "stable-save-setup-failure-v1"
                && v["name"] == f["name"]
                && v["targetStarted"] == false
                && v["scope"]
                    == "Observer initialization failed before any target command; diagnostic recorder may be unavailable",
            "StableV2 before-observer failure changed"
        );
        keys(&v["error"], &["name", "message"])?;
        error_text(&v["error"]["name"], 128)?;
        error_text(&v["error"]["message"], 1024)?;
        return Ok(());
    }
    keys(
        v,
        &[
            "protocol",
            "name",
            "phase",
            "targetStarted",
            "targetCommandsIssued",
            "targetProgress",
            "setupAuthorization",
            "combinedBudget",
            "progress",
            "originalTarget",
            "configurationDiagnostics",
            "error",
            "scope",
        ],
    )?;
    ensure!(
        v["protocol"] == "stable-save-setup-failure-v1"
            && v["name"] == f["name"]
            && v["scope"] == FAILURE_SCOPE,
        "StableV2 failure identity changed"
    );
    let phase = v["phase"]
        .as_str()
        .filter(|p| matches!(*p, "setup" | "target"))
        .ok_or_else(|| anyhow::anyhow!("StableV2 failure phase changed"))?;
    keys(&v["error"], &["name", "message"])?;
    error_text(&v["error"]["name"], 128)?;
    error_text(&v["error"]["message"], 1024)?;
    let prefix = &v["targetProgress"];
    optional(
        prefix,
        &[
            "protocol",
            "callbacks",
            "saved",
            "changes",
            "observations",
            "steps",
            "issued",
            "pending",
            "limits",
            "scope",
            "current",
        ],
        &["recordingError"],
    )?;
    compact(prefix, 524288)?;
    ensure!(
        prefix["protocol"] == "stable-save-target-prefix-v1"
            && prefix["scope"] == PREFIX_SCOPE
            && prefix["limits"]
                == json!({"maxRecordsPerList":256,"maxPrefixBytes":524288,"maxCurrentSnapshotBytes":163840,"maxCurrentTextBytes":65536,"maxCurrentSelections":256}),
        "StableV2 failure prefix protocol changed"
    );
    if let Some(e) = prefix.get("recordingError") {
        error_text(e, 1024)?;
    }
    let id = uri(
        &v["configurationDiagnostics"]["initial"]["uri"],
        f["name"].as_str().unwrap(),
        "main.txt",
    )?;
    event_lists(prefix)?;
    current(&prefix["current"], &id)?;
    keys(&v["originalTarget"], &["input", "disk", "diskError"])?;
    ensure!(
        v["originalTarget"]["input"] == TEXT
            && v["originalTarget"]["disk"] == prefix["current"]["disk"]
            && v["originalTarget"]["diskError"] == prefix["current"]["diskError"],
        "StableV2 failure original disk changed"
    );
    keys(
        &v["combinedBudget"],
        &["deadlineMs", "elapsedMs", "authorized"],
    )?;
    ensure!(
        v["combinedBudget"]["deadlineMs"] == 15000,
        "StableV2 failure setup deadline changed"
    );
    integer(&v["combinedBudget"]["elapsedMs"], 300000)?;
    let authorized = boolean(&v["combinedBudget"]["authorized"])?;
    ensure!(
        authorized != v["setupAuthorization"].is_null() && authorized == (phase == "target"),
        "StableV2 failure authorization changed"
    );
    if authorized {
        authorization(&v["setupAuthorization"])?;
    }
    let sequence = actions_sequence(f);
    let issued = list(&prefix["issued"], 256)?;
    let observations = list(&prefix["observations"], 256)?;
    let steps = list(&prefix["steps"], 256)?;
    ensure!(
        issued.len() <= sequence.len(),
        "StableV2 failure target sequence exceeded"
    );
    ensure!(
        issued.is_empty() || !observations.is_empty(),
        "StableV2 failure initial observation missing"
    );
    let mut count = 0;
    let mut completed = 0;
    for (i, r) in issued.iter().enumerate() {
        keys(r, &["sequence", "action", "commandIssued", "completed"])?;
        ensure!(
            r["sequence"] == json!(i + 1)
                && r["action"] == sequence[i]
                && r["commandIssued"] == json!(sequence[i] != "files.autoSave.afterDelay"),
            "StableV2 failure issued gesture changed"
        );
        count += usize::from(boolean(&r["commandIssued"])?);
        if boolean(&r["completed"])? {
            ensure!(completed == i, "StableV2 failure completed order changed");
            completed += 1;
        } else {
            ensure!(
                i + 1 == issued.len(),
                "StableV2 failure has multiple pending gestures"
            );
        }
    }
    ensure!(
        v["targetCommandsIssued"] == json!(count) && v["targetStarted"] == json!(count > 0),
        "StableV2 failure actual target count changed"
    );
    if completed < issued.len() {
        ensure!(
            prefix["pending"] == issued[completed],
            "StableV2 failure pending gesture changed"
        );
    } else {
        ensure!(
            prefix["pending"].is_null(),
            "StableV2 failure pending gesture changed"
        );
    }
    ensure!(
        steps.len() >= completed
            && steps.len() <= issued.len()
            && observations.len() <= issued.len() + 1
            && observations.len() >= completed + usize::from(!observations.is_empty()),
        "StableV2 failure completed prefix changed"
    );
    if phase == "setup" {
        ensure!(
            issued.is_empty() && observations.is_empty() && steps.is_empty(),
            "StableV2 setup failure started targets"
        );
    }
    for (i, o) in observations.iter().enumerate() {
        observed(o)?;
        ensure!(
            o["action"] == if i == 0 { "initial" } else { sequence[i - 1] },
            "StableV2 failure observation order changed"
        );
    }
    for (i, s) in steps.iter().enumerate() {
        step(s, sequence[i], list(&prefix["saved"], 256)?)?;
    }
    keys(
        &v["progress"],
        &["saveParticipantReadiness", "configurationReadiness"],
    )?;
    if !v["progress"]["saveParticipantReadiness"].is_null() {
        partial_participant(&v["progress"]["saveParticipantReadiness"], f, &id)?;
    }
    if !v["progress"]["configurationReadiness"].is_null() {
        configuration(&v["progress"]["configurationReadiness"], f, &id, false)?;
    }
    if authorized {
        participant(&v["progress"]["saveParticipantReadiness"], f, &id)?;
        configuration(&v["progress"]["configurationReadiness"], f, &id, true)?;
    }
    partial_diagnostics(&v["configurationDiagnostics"], prefix, &id, phase)?;
    if prefix.get("recordingError").is_none() {
        // Every changed completed model snapshot must have a retained public
        // change notification. Repeated/no-op observations need none.
        let changes = list(&prefix["changes"], 256)?;
        for pair in observations.windows(2) {
            if pair[0]["text"] != pair[1]["text"] {
                ensure!(
                    changes.iter().any(|e| e["text"] == pair[1]["text"]),
                    "StableV2 failure change prefix missing"
                );
            }
        }
        if prefix["current"]["status"] == "observed" {
            let events: Vec<_> = ["callbacks", "saved", "changes"]
                .iter()
                .flat_map(|k| prefix[*k].as_array().unwrap())
                .collect();
            if let Some(version) = events.iter().filter_map(|e| e["version"].as_u64()).max() {
                ensure!(
                    prefix["current"]["version"] == json!(version),
                    "StableV2 failure current version correlation changed"
                );
                for event in events
                    .into_iter()
                    .filter(|e| e["version"] == json!(version))
                {
                    ensure!(
                        event["text"] == prefix["current"]["text"],
                        "StableV2 failure current text correlation changed"
                    );
                }
            }
        }
    }
    Ok(())
}
fn failure_receipt(bytes: &[u8], proof: &Value, admitted: &Artifact, index: usize) -> Result<()> {
    ensure!(
        bytes.len() <= 2 * MIB,
        "StableV2 failure file budget exceeded"
    );
    let f = admitted
        .cases
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("StableV2 failure fixture changed"))?;
    provenance(
        proof,
        &admitted.manifest,
        &admitted.proof["platform"],
        &admitted.proof["architecture"],
        f,
        true,
    )?;
    ensure!(
        proof["failureSha256"] == sha(bytes),
        "StableV2 failure digest changed"
    );
    let original = &admitted.proof["runs"][index];
    for key in [
        "launcher",
        "productSha256",
        "profileSettingsSha256",
        "workspaceSettingsSha256",
    ] {
        ensure!(
            proof[key] == original[key],
            "StableV2 failure source receipt changed"
        );
    }
    failure(&parse(bytes)?, f)
}
fn archived(platform: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vscode-reference/observations/1.95.0/save-configuration-stable/1338970")
        .join(platform)
}
fn from_environment() -> Artifact {
    let root = std::env::var_os("VSCLI_SAVE_ACTIONS_STABLE_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| archived("linux"));
    Artifact::admit(&root).unwrap()
}
#[test]
fn genuine_archived_stable_v2_three_platform_metadata_preserves_every_frame() {
    for platform in ["linux", "darwin", "win32"] {
        let a = Artifact::admit(&archived(platform)).unwrap();
        assert_eq!(a.proof["platform"], platform);
        assert_eq!(a.raw.len(), 14);
        assert_eq!(a.trace.len(), 14);
    }
}

#[test]
#[ignore = "requires genuine remote StableV2 artifact; metadata only, no native target parity"]
fn genuine_stable_v2_complete_artifact_metadata() {
    let a = from_environment();
    assert_eq!(a.raw.len(), 14);
    assert_eq!(a.trace.len(), 14);
    assert!(a.root.is_dir());
    println!(
        "Validated {} StableV2 source/receipts/readiness/diagnostics:14cases162frames; native target parity was not asserted",
        a.proof["platform"]
    );
}
#[test]
fn quiet_clock_ties_and_earlier_success_cannot_invent_a_later_ready_window() {
    let samples = json!([{"elapsedMs":0,"matched":true},{"elapsedMs":100,"matched":true},{"elapsedMs":300,"matched":true}]);
    let rows = samples.as_array().unwrap();
    assert!(quiet_authorized(rows, &[], 0, 300));
    // A same-timestamp event can precede the first matching sample.
    assert!(quiet_authorized(rows, &[json!({"elapsedMs":0})], 0, 300));
    // A later event genuinely resets quiet even though every sampled value is
    // canonical. No preferred target state authorizes this proof.
    assert!(!quiet_authorized(rows, &[json!({"elapsedMs":200})], 0, 300));
    let mut extra = rows.to_vec();
    extra.push(json!({"elapsedMs":500,"matched":true}));
    assert!(!quiet_authorized(&extra, &[], 0, 500));
    let reset = json!([{"elapsedMs":0,"matched":true},{"elapsedMs":100,"matched":false},{"elapsedMs":200,"matched":true},{"elapsedMs":500,"matched":true}]);
    assert!(quiet_authorized(reset.as_array().unwrap(), &[], 200, 500));
}
#[test]
fn strict_json_duplicate_keys_and_actual_read_cap() {
    assert_eq!(
        parse(br#"{"a":1,"b":[true,null]}"#).unwrap(),
        json!({"a":1,"b":[true,null]})
    );
    assert!(
        parse(br#"{"a":1,"a":2}"#)
            .unwrap_err()
            .to_string()
            .contains("Duplicate")
    );
    assert!(parse(br#"{"a":{"x":1,"x":2}}"#).is_err());
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("input.json");
    fs::write(&file, b"12345").unwrap();
    assert_eq!(bounded(&file, 5).unwrap(), b"12345");
    assert_eq!(
        bounded(&file, 4).unwrap_err().to_string(),
        "StableV2 file budget exceeded"
    );
    assert_eq!(
        bounded(root.path(), 5).unwrap_err().to_string(),
        "StableV2 file budget exceeded"
    );
    #[cfg(unix)]
    {
        let link = root.path().join("link.json");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(
            bounded(&link, 5).unwrap_err().to_string(),
            "StableV2 file budget exceeded"
        );
    }
}
#[test]
fn genuine_stable_v2_metadata_mutations_reach_their_intended_guard() {
    let a = from_environment();
    let raw = &a.raw[0];
    let f = &a.cases[0];
    success(raw, f).unwrap();
    let check = |mutate: fn(&mut Value), expected: &str| {
        let mut v = raw.clone();
        mutate(&mut v);
        assert!(
            success(&v, f).unwrap_err().to_string().contains(expected),
            "Expected {expected}"
        );
    };
    check(
        |v| v["commandInventory"] = json!(["redo", "type", "undo"]),
        "command inventory",
    );
    check(|v| v["targetCommandsIssued"] = json!(13), "target count");
    check(
        |v| {
            v["setup"]["configurationReadiness"]["expected"]["effective"]["autoSaveDelay"] =
                json!(999)
        },
        "canonical declaration",
    );
    check(
        |v| v["setup"]["configurationReadiness"]["samples"][0]["matched"] = json!(false),
        "sample match",
    );
    check(
        |v| v["setup"]["configurationReadiness"]["quietSinceMs"] = json!(1),
        "quiet match",
    );
    check(
        |v| v["setup"]["setupAuthorization"]["elapsedMs"] = json!(15000),
        "integer",
    );
    check(
        |v| v["setup"]["saveParticipantReadiness"]["saveReason"] = json!("automatic"),
        "auxiliary configuration",
    );
    check(
        |v| v["setup"]["saveParticipantReadiness"]["callbacks"][0]["matched"] = json!(false),
        "match proof",
    );
    check(
        |v| v["setup"]["saveParticipantReadiness"]["callbacks"][0]["cancelled"] = json!(true),
        "match proof",
    );
    check(
        |v| v["setup"]["after"]["text"] = json!("changed"),
        "position",
    );
    check(
        |v| v["callbacks"][0]["returned"][0]["kind"] = json!("unknown"),
        "provider inventory",
    );
    check(
        |v| v["observations"][0]["resource"] = json!("elsewhere.txt"),
        "observed resource",
    );
    check(
        |v| v["stableConfiguration"]["targetChecks"][0]["matched"] = json!(false),
        "check correlation",
    );
    check(
        |v| {
            v["stableConfiguration"]["targetChecks"]
                .as_array_mut()
                .unwrap()
                .pop()
                .unwrap();
        },
        "check inventory",
    );
    check(
        |v| v["stableConfiguration"]["valid"] = json!(false),
        "valid flag",
    );
    for index in [7, 8] {
        let mut v = a.raw[index].clone();
        success(&v, &a.cases[index]).unwrap();
        v["setup"]["saveParticipantReadiness"]["targetAutoSave"] = json!("off");
        assert!(
            success(&v, &a.cases[index])
                .unwrap_err()
                .to_string()
                .contains("auxiliary configuration")
        );
    }
    for observer in [
        LEGACY_SAVE_OBSERVER,
        READY_SAVE_OBSERVER,
        DIAGNOSTIC_SAVE_OBSERVER,
    ] {
        assert_eq!(
            validate_save_observer_metadata(raw, observer)
                .unwrap_err()
                .to_string(),
            "Unexpected stable metadata in earlier observation"
        );
    }
    // Per-artifact source and receipt guards are tested without modifying any
    // genuine archive: derive separately labelled copied fixtures.
    let fixture = tempfile::tempdir().unwrap();
    for directory in [INPUT, OUTPUT] {
        fs::create_dir_all(fixture.path().join(directory)).unwrap();
    }
    for entry in fs::read_dir(a.root.join(INPUT)).unwrap() {
        let e = entry.unwrap();
        fs::copy(e.path(), fixture.path().join(INPUT).join(e.file_name())).unwrap();
    }
    for name in ["candidate-revision.txt", "capture.log"] {
        fs::copy(
            a.root.join(OUTPUT).join(name),
            fixture.path().join(OUTPUT).join(name),
        )
        .unwrap();
    }
    let result = fixture.path().join(OUTPUT).join("result");
    fs::create_dir(&result).unwrap();
    for entry in fs::read_dir(a.root.join(OUTPUT).join("result")).unwrap() {
        let e = entry.unwrap();
        fs::copy(e.path(), result.join(e.file_name())).unwrap();
    }
    Artifact::admit(fixture.path()).unwrap();
    let revised = fixture.path().join(OUTPUT).join("candidate-revision.txt");
    fs::write(&revised, b"unknown\n").unwrap();
    assert_eq!(
        Artifact::admit(fixture.path()).err().unwrap().to_string(),
        "StableV2 source revision changed"
    );
    fs::write(&revised, format!("{REVISION}\n")).unwrap();
    let manifest_path = fixture.path().join(INPUT).join("source-sha256.json");
    let bytes = fs::read(&manifest_path).unwrap();
    let mut m = a.manifest.clone();
    m.as_object_mut()
        .unwrap()
        .remove("save-configuration-ready.cjs");
    fs::write(&manifest_path, serde_json::to_vec(&m).unwrap()).unwrap();
    assert_eq!(
        Artifact::admit(fixture.path()).err().unwrap().to_string(),
        "StableV2 shape changed"
    );
    fs::write(&manifest_path, &bytes).unwrap();
    let extra = result.join("unrecorded-evidence.json");
    fs::write(&extra, b"{}\n").unwrap();
    assert_eq!(
        Artifact::admit(fixture.path()).err().unwrap().to_string(),
        "StableV2 result inventory changed"
    );
    fs::remove_file(extra).unwrap();
    let io = result.join("object-fixall-first-worker-io-provenance.json");
    let mut receipt = read(&io, 65536).unwrap();
    receipt["runtimeSettingsSha256"] = json!("0".repeat(64));
    fs::write(&io, serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert_eq!(
        Artifact::admit(fixture.path()).err().unwrap().to_string(),
        "StableV2 IO receipt changed"
    );
}

// Labeled synthetic failure, derived only after genuine full positive admission.
// It is not appended to any real archive or presented as an observation.
fn synthetic_failed_save(a: &Artifact) -> Value {
    let raw = &a.raw[0];
    let ready = &raw["setup"];
    let records: Vec<_> = raw["configurationDiagnostics"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["phase"] == "setup")
        .cloned()
        .collect();
    let callbacks: Vec<_> = raw["callbacks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["phase"] == "setup")
        .cloned()
        .collect();
    let changes: Vec<_> = raw["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["version"].as_u64().unwrap() <= 3)
        .cloned()
        .collect();
    let diag = json!({"protocol":"save-configuration-diagnostics-v1","limits":raw["configurationDiagnostics"]["limits"],"initial":raw["configurationDiagnostics"]["initial"],
        "final":records.last().map(|r|r["snapshot"].clone()).unwrap_or_else(||raw["configurationDiagnostics"]["initial"].clone()),"records":records,"status":"partial-failure","error":null,"scope":PARTIAL_SCOPE});
    let t = raw["observations"][1]["text"].as_str().unwrap();
    let line = t.split('\n').count() - 1;
    let col = t.split('\n').next_back().unwrap().encode_utf16().count();
    json!({"protocol":"stable-save-setup-failure-v1","name":raw["name"],"phase":"target","targetStarted":true,"targetCommandsIssued":2,
      "targetProgress":{"protocol":"stable-save-target-prefix-v1","callbacks":callbacks,"saved":[],"changes":changes,
        "observations":[raw["observations"][0],raw["observations"][1]],"steps":[raw["steps"][0]],
        "issued":[{"sequence":1,"action":"type","commandIssued":true,"completed":true},{"sequence":2,"action":"workbench.action.files.save","commandIssued":true,"completed":false}],
        "pending":{"sequence":2,"action":"workbench.action.files.save","commandIssued":true,"completed":false},
        "limits":{"maxRecordsPerList":256,"maxPrefixBytes":524288,"maxCurrentSnapshotBytes":163840,"maxCurrentTextBytes":65536,"maxCurrentSelections":256},"scope":PREFIX_SCOPE,
        "current":{"status":"observed","resource":"main.txt","uri":ready["configurationReadiness"]["uri"],"languageId":"plaintext","version":3,"text":t,"dirty":true,"selections":[[line,col,line,col]],"disk":TEXT,"diskError":null}},
      "setupAuthorization":ready["setupAuthorization"],"combinedBudget":{"deadlineMs":15000,"elapsedMs":16000,"authorized":true},
      "progress":{"saveParticipantReadiness":ready["saveParticipantReadiness"],"configurationReadiness":ready["configurationReadiness"]},
      "originalTarget":{"input":TEXT,"disk":TEXT,"diskError":null},"configurationDiagnostics":diag,
      "error":{"name":"Error","message":"Synthetic Save failure"},"scope":FAILURE_SCOPE})
}
#[test]
fn source_qualified_failure_prefix_preserves_pending_and_completed_evidence() {
    let a = from_environment();
    let f = &a.cases[0];
    let v = synthetic_failed_save(&a);
    failure(&v, f).unwrap();
    let check = |mutate: fn(&mut Value), needle: &str| {
        let mut v = v.clone();
        mutate(&mut v);
        assert!(failure(&v, f).unwrap_err().to_string().contains(needle));
    };
    check(
        |v| v["targetCommandsIssued"] = json!(1),
        "actual target count",
    );
    check(
        |v| v["targetProgress"]["pending"]["action"] = json!("redo"),
        "pending gesture",
    );
    check(
        |v| {
            v["targetProgress"]["observations"]
                .as_array_mut()
                .unwrap()
                .pop()
                .unwrap();
        },
        "completed prefix",
    );
    check(
        |v| v["targetProgress"]["callbacks"] = json!([]),
        "callback missing",
    );
    check(
        |v| v["targetProgress"]["changes"] = json!([]),
        "change prefix missing",
    );

    check(
        |v| v["targetProgress"]["current"]["text"] = json!("x".repeat(65537)),
        "string budget",
    );
    check(
        |v| v["error"]["message"] = json!("x".repeat(1025)),
        "error budget",
    );
    check(
        |v| v["originalTarget"]["disk"] = json!("other"),
        "original disk",
    );
    check(
        |v| v["combinedBudget"]["authorized"] = json!(false),
        "authorization",
    );
    let mut unavailable = v.clone();
    unavailable["targetProgress"]["current"] = json!({"status":"unavailable","error":"Synthetic getText failure","disk":null,"diskError":"Synthetic deleted disk"});
    unavailable["originalTarget"]["disk"] = Value::Null;
    unavailable["originalTarget"]["diskError"] = json!("Synthetic deleted disk");
    failure(&unavailable, f).unwrap();
    assert_eq!(unavailable["error"]["message"], "Synthetic Save failure");
    let mut setup = v.clone();
    setup["phase"] = json!("setup");
    setup["targetStarted"] = json!(false);
    setup["targetCommandsIssued"] = json!(0);
    setup["setupAuthorization"] = Value::Null;
    setup["combinedBudget"]["authorized"] = json!(false);
    setup["targetProgress"]["issued"] = json!([]);
    setup["targetProgress"]["pending"] = Value::Null;
    setup["targetProgress"]["observations"] = json!([]);
    setup["targetProgress"]["steps"] = json!([]);
    setup["targetProgress"]["callbacks"] = json!([]);
    setup["targetProgress"]["changes"] = json!([]);
    setup["targetProgress"]["current"] = json!({"status":"unavailable","error":"Target document/editor not bound","disk":TEXT,"diskError":null});

    setup["progress"]["configurationReadiness"] = Value::Null;
    setup["progress"]["saveParticipantReadiness"] = json!({"protocol":"save-participant-readiness-v2","status":"failed","resource":"readiness.ini","input":AUX_INPUT,"saveReason":"explicit-auxiliary","targetAutoSave":"off","limits":{"maxAttempts":16,"deadlineMs":5000,"retryIntervalMs":250},"attempts":[],"callbacks":[],"error":"Synthetic initial open timeout"});
    setup["configurationDiagnostics"]["records"] = json!([]);
    setup["configurationDiagnostics"]["final"] =
        setup["configurationDiagnostics"]["initial"].clone();
    failure(&setup, f).unwrap();
    let mut proof = a.proof["runs"][0].clone();
    proof.as_object_mut().unwrap().remove("evidenceSha256");
    let bytes = serde_json::to_vec(&v).unwrap();
    proof["failureSha256"] = json!(sha(&bytes));
    proof["scope"] = json!(FAILURE_PROVENANCE_SCOPE);
    failure_receipt(&bytes, &proof, &a, 0).unwrap();
    let mut invalid = proof.clone();
    invalid["failureSha256"] = json!("0".repeat(64));
    assert_eq!(
        failure_receipt(&bytes, &invalid, &a, 0)
            .unwrap_err()
            .to_string(),
        "StableV2 failure digest changed"
    );
    invalid = proof.clone();
    invalid["sources"]["save-configuration-ready.cjs"] = json!("0".repeat(64));
    assert_eq!(
        failure_receipt(&bytes, &invalid, &a, 0)
            .unwrap_err()
            .to_string(),
        "StableV2 per-case sources changed"
    );
}
