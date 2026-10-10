//! Explicit admission context for an isolated current artifact. Candidate receipt
//! bytes never choose the trusted revision, and current admission does not relax
//! any archived envelope/source/metadata rule.
use super::*;
use std::ffi::OsString;

const ARTIFACT_ENV: &str = "VSCLI_SAVE_ACTIONS_STABLE_CURRENT_ARTIFACT_DIR";
const REVISION_ENV: &str = "VSCLI_SAVE_ACTIONS_STABLE_EXPECTED_REVISION";

pub(super) enum Context<'a> {
    Archived133,
    CurrentCapture { trusted_revision: &'a str },
}
impl<'a> Context<'a> {
    pub(super) fn expected_revision(self) -> Result<&'a str> {
        match self {
            Self::Archived133 => Ok(REVISION),
            Self::CurrentCapture { trusted_revision } => {
                ensure!(
                    trusted_revision.len() == 40
                        && trusted_revision
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "StableV2 trusted current revision must be lowercase 40-hex"
                );
                Ok(trusted_revision)
            }
        }
    }
}

fn current_or_archived(
    directory: Option<OsString>,
    revision: Option<OsString>,
) -> Result<Artifact> {
    match (directory, revision) {
        (None, None) => Artifact::admit(&archived("linux")),
        (Some(directory), Some(revision)) => {
            ensure!(
                !directory.is_empty(),
                "StableV2 current artifact directory must be nonempty"
            );
            let revision = revision.into_string().map_err(|_| {
                anyhow::anyhow!("StableV2 trusted current revision must be lowercase 40-hex")
            })?;
            Artifact::admit_with_context(
                &PathBuf::from(directory),
                Context::CurrentCapture {
                    trusted_revision: &revision,
                },
            )
        }
        _ => anyhow::bail!(
            "StableV2 current artifact directory and trusted revision must both be provided"
        ),
    }
}

pub(super) fn current_corpus() -> Result<Vec<Value>> {
    Ok(current_or_archived(
        std::env::var_os(ARTIFACT_ENV),
        std::env::var_os(REVISION_ENV),
    )?
    .trace)
}

// These are separately labeled copied receipts, not fresh observations. Only
// candidate-revision.txt changes for the positive context test; source, result,
// product provenance and all target-once/readiness proofs remain genuine.
const SYNTHETIC_CURRENT: &str = "0123456789abcdef0123456789abcdef01234567";

fn copied_current_fixture() -> tempfile::TempDir {
    let genuine = Artifact::admit(&archived("linux")).unwrap();
    let copied = tempfile::tempdir().unwrap();
    let input = copied.path().join(INPUT);
    let output = copied.path().join(OUTPUT);
    let result = output.join("result");
    fs::create_dir_all(&input).unwrap();
    fs::create_dir_all(&result).unwrap();
    for entry in fs::read_dir(genuine.root.join(INPUT)).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), input.join(entry.file_name())).unwrap();
    }
    for name in ["candidate-revision.txt", "capture.log"] {
        fs::copy(genuine.root.join(OUTPUT).join(name), output.join(name)).unwrap();
    }
    for entry in fs::read_dir(genuine.root.join(OUTPUT).join("result")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), result.join(entry.file_name())).unwrap();
    }
    // Positive admission precedes any mutation of the independently copied data.
    Artifact::admit(copied.path()).unwrap();
    fs::write(
        output.join("candidate-revision.txt"),
        format!("{SYNTHETIC_CURRENT}\n"),
    )
    .unwrap();
    admit_current(copied.path(), SYNTHETIC_CURRENT).unwrap();
    copied
}
fn admit_current(path: &Path, expected: &str) -> Result<Artifact> {
    Artifact::admit_with_context(
        path,
        Context::CurrentCapture {
            trusted_revision: expected,
        },
    )
}
fn error(result: Result<Artifact>) -> String {
    result
        .err()
        .expect("Mutation must refuse admission")
        .to_string()
}

#[test]
fn current_context_authorizes_only_the_independently_supplied_exact_revision() {
    let copied = copied_current_fixture();
    let current = admit_current(copied.path(), SYNTHETIC_CURRENT).unwrap();
    assert_eq!(current.cases.len(), 14);
    assert_eq!(current.raw.len(), 14);
    assert_eq!(current.trace.len(), 14);
    assert_eq!(
        current
            .trace
            .iter()
            .map(|row| row["observations"].as_array().unwrap().len())
            .sum::<usize>(),
        162
    );
    assert_eq!(
        error(Artifact::admit(copied.path())),
        "StableV2 source revision changed"
    );
    assert_eq!(
        error(admit_current(copied.path(), REVISION)),
        "StableV2 source revision changed"
    );
    for invalid in [
        "",
        "unknown",
        "0123456789abcdef0123456789abcdef0123456",
        "0123456789abcdef0123456789abcdef012345678",
        "0123456789ABCDEF0123456789abcdef01234567",
        "g123456789abcdef0123456789abcdef01234567",
        "0123456789abcdef0123456789abcdef01234567\n",
    ] {
        assert_eq!(
            error(admit_current(copied.path(), invalid)),
            "StableV2 trusted current revision must be lowercase 40-hex"
        );
    }
    // Restoring the archive receipt positively re-admits under the old context.
    fs::write(
        copied.path().join(OUTPUT).join("candidate-revision.txt"),
        format!("{REVISION}\n"),
    )
    .unwrap();
    Artifact::admit(copied.path()).unwrap();
}

#[test]
fn current_context_preserves_exact_runtime_receipt_and_source_guards() {
    let copied = copied_current_fixture();
    let input = copied.path().join(INPUT);
    let result = copied.path().join(OUTPUT).join("result");
    let runtime = result.join("object-fixall-first-runtime-settings.json");
    let original = fs::read(&runtime).unwrap();
    let mut changed = parse(&original).unwrap();
    let settings = changed["profileSettings"].as_str().unwrap().to_owned();
    changed["profileSettings"] = json!(format!("{settings} "));
    fs::write(&runtime, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert_eq!(
        error(admit_current(copied.path(), SYNTHETIC_CURRENT)),
        "StableV2 IO digests changed"
    );
    fs::write(&runtime, original).unwrap();
    admit_current(copied.path(), SYNTHETIC_CURRENT).unwrap();

    let receipt = result.join("object-fixall-first-worker-io-provenance.json");
    let original = fs::read(&receipt).unwrap();
    fs::remove_file(&receipt).unwrap();
    assert_eq!(
        error(admit_current(copied.path(), SYNTHETIC_CURRENT)),
        "StableV2 result inventory changed"
    );
    fs::write(&receipt, original).unwrap();
    admit_current(copied.path(), SYNTHETIC_CURRENT).unwrap();

    let source = input.join("extension.cjs");
    let original = fs::read(&source).unwrap();
    let mut changed = original.clone();
    changed.extend_from_slice(b"\n// separately labeled test mutation\n");
    fs::write(&source, changed).unwrap();
    assert_eq!(
        error(admit_current(copied.path(), SYNTHETIC_CURRENT)),
        "StableV2 source bytes changed"
    );
    fs::write(&source, original).unwrap();
    admit_current(copied.path(), SYNTHETIC_CURRENT).unwrap();
}

#[test]
fn current_context_keeps_exact_outer_inner_and_result_inventories() {
    let copied = copied_current_fixture();
    for (directory, intended_guard) in [
        (
            copied.path().to_owned(),
            "StableV2 directory inventory exceeds bounds",
        ),
        (
            copied.path().join("tests"),
            "StableV2 directory inventory exceeds bounds",
        ),
        (
            copied.path().join("target"),
            "StableV2 directory inventory exceeds bounds",
        ),
        (
            copied.path().join(INPUT),
            "StableV2 directory inventory exceeds bounds",
        ),
        (
            copied.path().join(OUTPUT),
            "StableV2 directory inventory exceeds bounds",
        ),
        (
            copied.path().join(OUTPUT).join("result"),
            "StableV2 result inventory changed",
        ),
    ] {
        let extra = directory.join("unrecorded-extra.json");
        fs::write(&extra, b"{}\n").unwrap();
        assert_eq!(
            error(admit_current(copied.path(), SYNTHETIC_CURRENT)),
            intended_guard
        );
        fs::remove_file(extra).unwrap();
        admit_current(copied.path(), SYNTHETIC_CURRENT).unwrap();
    }
}

#[test]
fn current_environment_requires_the_explicit_pair_without_global_env_mutation() {
    let copied = copied_current_fixture();
    let directory = copied.path().as_os_str().to_owned();
    let revision = OsString::from(SYNTHETIC_CURRENT);
    let current = current_or_archived(Some(directory.clone()), Some(revision.clone())).unwrap();
    assert_eq!(current.root, copied.path());
    for (directory, revision) in [(Some(directory), None), (None, Some(revision))] {
        assert_eq!(
            error(current_or_archived(directory, revision)),
            "StableV2 current artifact directory and trusted revision must both be provided"
        );
    }
    assert_eq!(
        error(current_or_archived(
            Some(OsString::new()),
            Some(OsString::from(SYNTHETIC_CURRENT))
        )),
        "StableV2 current artifact directory must be nonempty"
    );
    assert_eq!(
        current_or_archived(None, None).unwrap().root,
        archived("linux")
    );
    // Exercise the parent-facing production entry point without modifying any
    // global environment. CI can supply its admitted explicit current pair;
    // ordinary local tests use the archived cohort.
    assert_eq!(super::current_corpus().unwrap().len(), 14);
}

#[test]
#[ignore = "requires explicit isolated current StableV2 artifact and independently trusted revision; metadata only"]
fn genuine_current_stable_v2_complete_artifact_metadata() {
    let directory = std::env::var_os(ARTIFACT_ENV);
    let revision = std::env::var_os(REVISION_ENV);
    assert!(
        directory.is_some() && revision.is_some(),
        "Current metadata qualification requires both explicit environment inputs"
    );
    let admitted = current_or_archived(directory, revision).unwrap();
    assert_eq!(admitted.raw.len(), 14);
    assert_eq!(admitted.trace.len(), 14);
    assert!(admitted.root.is_dir());
    assert_eq!(super::current_corpus().unwrap(), admitted.trace);
    println!(
        "Validated {} current StableV2 fixed sources/receipts/readiness/diagnostics:14cases162frames; native target parity was not asserted",
        admitted.proof["platform"]
    );
}
