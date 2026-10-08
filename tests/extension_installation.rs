use serde_json::json;
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    process::Command,
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
fn cli_install_list_rollback_uninstall_work_without_terminal_or_node() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("package.vsix");
    let storage = temp.path().join("store");
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_vscli"))
            .arg("--extensions-dir")
            .arg(&storage)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    package(&archive, "1.0.0", &[]);
    assert!(
        run(&["--install-extension", archive.to_str().unwrap()])
            .contains("Installed example.command@1.0.0")
    );
    package(&archive, "2.0.0", &[]);
    run(&["--install-extension", archive.to_str().unwrap()]);
    assert!(run(&["--rollback-extension", "example.command"]).contains("1.0.0"));
    let listed: serde_json::Value = serde_json::from_str(&run(&["--list-extensions"])).unwrap();
    assert_eq!(listed[0]["id"], "example.command");
    run(&["--uninstall-extension", "example.command"]);
    assert_eq!(run(&["--list-extensions"]).trim(), "[]");
}

#[test]
#[ignore = "requires the pinned compiled Sort Lines package and Node; real protocol CI runs this"]
fn upstream_sort_lines_vsix_installs_and_runs_without_source_changes() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use std::time::{Duration, Instant};
    use vscli::{app::App, extensions::Client, keys::Profile};
    let source = std::path::PathBuf::from(
        std::env::var_os("VSCLI_TEST_SORT_LINES").expect("Set VSCLI_TEST_SORT_LINES"),
    );
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("sort-lines.vsix");
    let mut zip = ZipWriter::new(File::create(&archive).unwrap());
    for name in ["package.json", "out/extension.js", "out/sort-lines.js"] {
        zip.start_file(format!("extension/{name}"), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&fs::read(source.join(name)).unwrap())
            .unwrap();
    }
    zip.finish().unwrap();
    let installed = Store::new(temp.path().join("extensions"))
        .install(&archive)
        .unwrap();
    assert_eq!(installed.id, "tyriar.sort-lines");
    assert_eq!(installed.version, "1.12.0");
    let path = temp.path().join("lines.txt");
    fs::write(&path, "zebra\napple\npear").unwrap();
    let mut app = App::new(temp.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    app.execute("editor.action.selectAll", serde_json::Value::Null);
    app.extension_host = Some(
        Client::start(
            "node",
            &installed.path,
            &app.workspace.root,
            &app.documents,
            app.active,
            &app.settings,
        )
        .unwrap(),
    );
    let started = Instant::now();
    while !app.extension_host.as_ref().is_some_and(|host| host.ready) {
        app.poll();
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "{}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    app.event(Event::Key(KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE)));
    while app.doc().text != "apple\npear\nzebra" {
        app.poll();
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "{}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), "zebra\napple\npear");
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "zebra\napple\npear");
}

#[test]
fn original_extensions_shortcut_opens_installed_picker_on_all_profiles() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use std::time::{Duration, Instant};
    use vscli::{
        app::{App, Modal},
        keys::Profile,
    };
    let temp = tempfile::tempdir().unwrap();
    for (profile, modifier) in [
        (Profile::Linux, KeyModifiers::CONTROL),
        (Profile::Windows, KeyModifiers::CONTROL),
        (Profile::Macos, KeyModifiers::SUPER),
    ] {
        let mut app = App::new(temp.path().into(), profile);
        app.extensions_directory = Some(temp.path().join("empty-store"));
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            modifier | KeyModifiers::SHIFT,
        )));
        let started = Instant::now();
        while !matches!(app.modal, Some(Modal::Extensions { .. })) {
            app.poll();
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "{}",
                app.message
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(app.documents.is_empty());
        assert!(app.extension_host.is_none());
    }
}
