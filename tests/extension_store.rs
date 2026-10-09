use serde_json::json;
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};
use vscli::extension_store::Store;
use zip::{ZipWriter, write::SimpleFileOptions};

fn package(path: &Path, version: &str, extra: &[(&str, &str)]) {
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    zip.start_file("extension/package.json", SimpleFileOptions::default())
        .unwrap();
    write!(zip, "{}", json!({"publisher":"Example","name":"command","version":version,"main":"main.cjs","engines":{"vscode":"^1.95.0"}})).unwrap();
    zip.start_file("extension/main.cjs", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"exports.activate=()=>{};").unwrap();
    for (name, content) in extra {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn install_upgrade_rollback_and_uninstall_preserve_immutable_generations() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("command.vsix");
    let store = Store::new(temp.path().join("store"));
    package(&archive, "1.0.0", &[]);
    let original = store.install(&archive).unwrap();
    assert_eq!(original.id, "example.command");
    assert!(original.compatibility.contains("Experimental"));
    assert_eq!(original.sha256.len(), 64);
    assert!(original.source.ends_with("command.vsix"));
    package(&archive, "2.0.0", &[]);
    let upgraded = store.install(&archive).unwrap();
    assert_ne!(original.path, upgraded.path);
    assert_ne!(original.sha256, upgraded.sha256);
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(store.get("EXAMPLE.COMMAND").unwrap().version, "2.0.0");
    assert_eq!(
        store.rollback("example.command").unwrap().path,
        original.path
    );
    assert_eq!(
        store.rollback("example.command").unwrap().path,
        upgraded.path
    );
    store.uninstall("example.command").unwrap();
    assert!(store.list().unwrap().is_empty());
    assert!(original.path.join("main.cjs").exists());
    assert!(upgraded.path.join("main.cjs").exists());
}

#[test]
fn malicious_archives_never_replace_the_installed_generation_or_escape_staging() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("package.vsix");
    let store = Store::new(temp.path().join("store"));
    package(&archive, "1.0.0", &[]);
    let original = store.install(&archive).unwrap();
    let registry = fs::read(store.root().join("registry.json")).unwrap();
    for name in [
        "../escape",
        "/absolute",
        "extension/../../escape",
        "extension\\escape",
        "extension/AUX.txt",
        "extension/C:escape",
        "extension/trailing.",
        "extension/MAIN.CJS",
    ] {
        package(&archive, "2.0.0", &[(name, "bad")]);
        assert!(store.install(&archive).is_err(), "accepted {name}");
        assert_eq!(
            fs::read(store.root().join("registry.json")).unwrap(),
            registry
        );
        assert_eq!(store.get(&original.id).unwrap().path, original.path);
        assert!(!temp.path().join("escape").exists());
        assert!(fs::read_dir(store.root()).unwrap().all(|p| {
            !p.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".stage-")
        }));
    }
}

#[test]
fn symlink_payload_is_rejected_without_following_it() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("link.vsix");
    let mut zip = ZipWriter::new(File::create(&archive).unwrap());
    zip.add_symlink(
        "extension/link",
        "../../outside",
        SimpleFileOptions::default(),
    )
    .unwrap();
    zip.finish().unwrap();
    let store = Store::new(temp.path().join("store"));
    assert!(
        store
            .install(&archive)
            .unwrap_err()
            .to_string()
            .contains("Links")
    );
    assert!(!store.root().join("registry.json").exists());
}

#[test]
fn oversized_payload_and_invalid_manifest_leave_registry_intact() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("bad.vsix");
    let store = Store::new(temp.path().join("store"));
    package(&archive, "1.0.0", &[]);
    store.install(&archive).unwrap();
    let before = fs::read(store.root().join("registry.json")).unwrap();
    let mut zip = ZipWriter::new(File::create(&archive).unwrap());
    zip.start_file(
        "extension/large",
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
    )
    .unwrap();
    let block = [0u8; 65536];
    for _ in 0..513 {
        zip.write_all(&block).unwrap();
    }
    zip.finish().unwrap();
    assert!(
        store
            .install(&archive)
            .unwrap_err()
            .to_string()
            .contains("32 MiB")
    );
    let mut zip = ZipWriter::new(File::create(&archive).unwrap());
    zip.start_file("extension/package.json", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"{ invalid manifest").unwrap();
    zip.finish().unwrap();
    assert!(store.install(&archive).is_err());
    fs::write(&archive, "not a zip").unwrap();
    assert!(store.install(&archive).is_err());
    assert_eq!(
        fs::read(store.root().join("registry.json")).unwrap(),
        before
    );
}

#[test]
fn operation_lock_and_corrupt_registry_preserve_installed_files() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("package.vsix");
    let store = Store::new(temp.path().join("store"));
    package(&archive, "1.0.0", &[]);
    let installed = store.install(&archive).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(store.root().join(".lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(
        store
            .install(&archive)
            .unwrap_err()
            .to_string()
            .contains("Another extension operation")
    );
    drop(lock);
    fs::write(store.root().join("registry.json"), "{ broken").unwrap();
    assert!(store.install(&archive).is_err());
    assert_eq!(
        fs::read_to_string(store.root().join("registry.json")).unwrap(),
        "{ broken"
    );
    assert!(installed.path.join("main.cjs").exists());
}

#[test]
fn duplicate_names_and_unbounded_central_directory_counts_are_rejected_before_extraction() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("malformed.vsix");
    let store = Store::new(temp.path().join("store"));
    package(&archive, "1.0.0", &[]);
    let original = store.install(&archive).unwrap();
    package(
        &archive,
        "2.0.0",
        &[("extension/dupa", "same"), ("extension/dupb", "same")],
    );
    let mut bytes = fs::read(&archive).unwrap();
    for index in 0..bytes.len() - 3 {
        if &bytes[index..index + 4] == b"dupb" {
            bytes[index + 3] = b'a';
        }
    }
    fs::write(&archive, &bytes).unwrap();
    assert!(
        store
            .install(&archive)
            .unwrap_err()
            .to_string()
            .contains("Duplicate")
    );
    package(&archive, "2.0.0", &[]);
    let mut bytes = fs::read(&archive).unwrap();
    let end = bytes.len() - 22;
    bytes[end + 8..end + 10].copy_from_slice(&30_000u16.to_le_bytes());
    bytes[end + 10..end + 12].copy_from_slice(&30_000u16.to_le_bytes());
    fs::write(&archive, &bytes).unwrap();
    assert!(
        store
            .install(&archive)
            .unwrap_err()
            .to_string()
            .contains("20,000")
    );
    assert_eq!(store.get(&original.id).unwrap().path, original.path);
}

#[test]
fn cumulative_manifest_budget_limits_listing_without_disabling_direct_ids() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("metadata.vsix");
    let store = Store::new(temp.path().join("store"));
    for name in ["first", "second"] {
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file("extension/package.json", SimpleFileOptions::default())
            .unwrap();
        write!(
            zip,
            "{}",
            json!({"publisher":"example","name":name,"version":"1.0.0","metadata":vec![0; 30_000]})
        )
        .unwrap();
        zip.finish().unwrap();
        store.install(&archive).unwrap();
    }
    assert!(
        store
            .list()
            .unwrap_err()
            .to_string()
            .contains("metadata budget")
    );
    assert_eq!(store.get("example.first").unwrap().id, "example.first");
    store.uninstall("example.first").unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn malformed_final_footer_cannot_fall_back_to_an_unbounded_earlier_footer() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("footer.vsix");
    package(&archive, "1.0.0", &[]);
    let mut bytes = fs::read(&archive).unwrap();
    let old_end = bytes.len() - 22;
    bytes[old_end + 8..old_end + 10].copy_from_slice(&30_000u16.to_le_bytes());
    bytes[old_end + 10..old_end + 12].copy_from_slice(&30_000u16.to_le_bytes());
    let mut misleading_footer = [0u8; 22];
    misleading_footer[..4].copy_from_slice(b"PK\x05\x06");
    misleading_footer[8..10].copy_from_slice(&1u16.to_le_bytes());
    misleading_footer[10..12].copy_from_slice(&1u16.to_le_bytes());
    misleading_footer[12..16].copy_from_slice(&22u32.to_le_bytes());
    misleading_footer[16..20].copy_from_slice(&(old_end as u32).to_le_bytes());
    bytes.extend(misleading_footer);
    fs::write(&archive, bytes).unwrap();
    let store = Store::new(temp.path().join("store"));
    let error = store.install(&archive).unwrap_err();
    assert!(
        format!("{error:#}").contains("Unexpected VSIX end-of-directory"),
        "{error:#}"
    );
    assert!(!store.root().join("registry.json").exists());
}
