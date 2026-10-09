use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use vscli::{
    extension_registry::{Entry, Registry, target_platform},
    extension_store::Store,
};
use zip::{ZipWriter, write::SimpleFileOptions};

type Routes = Arc<Mutex<HashMap<String, (u16, Vec<u8>)>>>;

struct Server {
    url: String,
    routes: Routes,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let routes = Arc::new(Mutex::new(HashMap::<String, (u16, Vec<u8>)>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let entries = routes.clone();
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                };
                // Accepted sockets can inherit nonblocking mode on macOS/Windows.
                // The owned fixture worker must wait for the complete request.
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut data = Vec::new();
                let mut byte = [0];
                while data.len() < 8192 && !data.ends_with(b"\r\n\r\n") {
                    if socket.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    data.push(byte[0]);
                }
                let text = String::from_utf8_lossy(&data);
                let path = text.split_whitespace().nth(1).unwrap_or("");
                let (status, body) = entries
                    .lock()
                    .unwrap()
                    .get(path)
                    .cloned()
                    .unwrap_or((404, Vec::new()));
                let header = format!(
                    "HTTP/1.1 {status} response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes());
                let _ = socket.write_all(&body);
            }
        });
        Self {
            url,
            routes,
            stop,
            worker: Some(worker),
        }
    }
    fn put(&self, path: &str, status: u16, body: Vec<u8>) {
        self.routes
            .lock()
            .unwrap()
            .insert(path.into(), (status, body));
    }
    fn metadata(&self, version: &str) -> Value {
        json!({"namespace":"fixture","name":"command","displayName":"Fixture", "description":"native command", "version":version, "license":"MIT", "targetPlatform":"universal", "files":{"download":format!("{}/download", self.url), "sha256":format!("{}/checksum",self.url)}})
    }
    fn latest(&self, version: &str) {
        self.put(
            "/api/fixture/command/universal/latest",
            200,
            serde_json::to_vec(&self.metadata(version)).unwrap(),
        );
    }
    fn archive(&self, publisher: &str, version: &str) {
        let mut archive = ZipWriter::new(std::io::Cursor::new(Vec::new()));
        archive
            .start_file("extension/package.json", SimpleFileOptions::default())
            .unwrap();
        write!(
            archive,
            "{}",
            json!({"publisher":publisher,"name":"command","version":version,"main":"index.cjs"})
        )
        .unwrap();
        archive
            .start_file("extension/index.cjs", SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"exports.activate=()=>{};").unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        use sha2::{Digest, Sha256};
        self.put(
            "/checksum",
            200,
            format!("{:x} fixture.vsix\n", Sha256::digest(&bytes)).into_bytes(),
        );
        self.put("/download", 200, bytes);
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn download_checks_digest_and_identity_before_atomic_install_and_retains_rollback() {
    let server = Server::new();
    let registry = Registry::new(&server.url).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().join("extensions"));
    let cancel = AtomicBool::new(false);
    server.latest("1.0.0");
    server.archive("fixture", "1.0.0");
    let entry = registry.latest("fixture.command", &cancel).unwrap();
    let original = registry.install(&entry, &store, &cancel).unwrap();
    assert_eq!(original.source, format!("{}/download", server.url));
    let bytes = std::fs::read(store.root().join("registry.json")).unwrap();
    server.latest("2.0.0");
    server.archive("fixture", "2.0.0");
    let newer = registry.latest("fixture.command", &cancel).unwrap();
    server.put("/checksum", 200, "0".repeat(64).into_bytes());
    assert!(
        registry
            .install(&newer, &store, &cancel)
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    assert_eq!(
        std::fs::read(store.root().join("registry.json")).unwrap(),
        bytes
    );
    server.archive("different", "2.0.0");
    assert!(
        registry
            .install(&newer, &store, &cancel)
            .unwrap_err()
            .to_string()
            .contains("identity/version")
    );
    assert_eq!(
        std::fs::read(store.root().join("registry.json")).unwrap(),
        bytes
    );
    server.archive("fixture", "1.0.0");
    assert!(registry.install(&newer, &store, &cancel).is_err());
    assert_eq!(store.get("fixture.command").unwrap().path, original.path);
    server.archive("fixture", "2.0.0");
    let installed = registry.install(&newer, &store, &cancel).unwrap();
    assert_ne!(installed.path, original.path);
    assert_eq!(
        store.rollback("fixture.command").unwrap().path,
        original.path
    );
    assert!(installed.path.join("index.cjs").is_file());
    assert!(original.path.join("index.cjs").is_file());
}

#[test]
fn stable_updates_prefer_host_platform_and_never_offer_downgrades() {
    let server = Server::new();
    let registry = Registry::new(&server.url).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().join("extensions"));
    let cancel = AtomicBool::new(false);
    server.latest("2.0.0");
    server.archive("fixture", "2.0.0");
    registry
        .install(
            &registry.latest("fixture.command", &cancel).unwrap(),
            &store,
            &cancel,
        )
        .unwrap();
    server.latest("1.0.0");
    assert!(
        registry
            .updates(&store.list().unwrap(), &cancel)
            .unwrap()
            .items
            .is_empty()
    );
    server.latest("3.0.0");
    let updates = registry.updates(&store.list().unwrap(), &cancel).unwrap();
    assert_eq!(updates.items[0].version, "3.0.0");
    let mut installed = store.list().unwrap();
    let mut unavailable = installed[0].clone();
    unavailable.id = "private.absent".into();
    installed.push(unavailable);
    let report = registry.updates(&installed, &cancel).unwrap();
    assert_eq!(report.items[0].version, "3.0.0");
    assert_eq!(report.notices.len(), 1);
    assert!(report.notices[0].contains("private.absent"));
    assert!(
        registry
            .updates(&vec![installed[0].clone(); 129], &cancel)
            .is_err()
    );
    let mut entry = server.metadata("4.0.0");
    entry["targetPlatform"] = target_platform().into();
    server.put(
        &format!("/api/fixture/command/{}/latest", target_platform()),
        200,
        serde_json::to_vec(&entry).unwrap(),
    );
    assert_eq!(
        registry.latest("fixture.command", &cancel).unwrap().version,
        "4.0.0"
    );
    entry["targetPlatform"] = "web".into();
    server.put(
        &format!("/api/fixture/command/{}/latest", target_platform()),
        200,
        serde_json::to_vec(&entry).unwrap(),
    );
    assert!(registry.latest("fixture.command", &cancel).is_err());
}

#[test]
fn search_url_encodes_query_and_rejects_oversized_or_unsupported_results() {
    let server = Server::new();
    let registry = Registry::new(&server.url).unwrap();
    let cancel = AtomicBool::new(false);
    let path = "/api/-/search?query=a%26b+%F0%9F%98%80&size=20";
    server.put(
        path,
        200,
        serde_json::to_vec(&json!({"extensions":[server.metadata("1.0.0")]})).unwrap(),
    );
    assert_eq!(
        registry.search("a&b 😀", &cancel).unwrap()[0].id,
        "fixture.command"
    );
    server.put(
        path,
        200,
        serde_json::to_vec(&json!({"extensions":vec![server.metadata("1.0.0");21]})).unwrap(),
    );
    assert!(
        registry
            .search("a&b 😀", &cancel)
            .unwrap_err()
            .to_string()
            .contains("20 results")
    );
    let mut entry = server.metadata("1.0.0-beta");
    server.put(
        path,
        200,
        serde_json::to_vec(&json!({"extensions":[entry]})).unwrap(),
    );
    assert!(registry.search("a&b 😀", &cancel).unwrap().is_empty());
    entry = server.metadata("1.0.0");
    entry["description"] = "x".repeat(4097).into();
    server.put(
        path,
        200,
        serde_json::to_vec(&json!({"extensions":[entry]})).unwrap(),
    );
    assert!(registry.search("a&b 😀", &cancel).is_err());
    let mut summary = server.metadata("1.0.0");
    summary.as_object_mut().unwrap().remove("targetPlatform");
    summary["files"]["download"] = "https://wrong.invalid/wrong-platform.vsix".into();
    server.put(
        path,
        200,
        serde_json::to_vec(&json!({"extensions":[summary]})).unwrap(),
    );
    server.put(
        "/api/fixture/command/universal/1.0.0",
        200,
        serde_json::to_vec(&server.metadata("1.0.0")).unwrap(),
    );
    let resolved = registry.search("a&b 😀", &cancel).unwrap();
    assert_eq!(resolved[0].platform, "universal");
    assert_eq!(resolved[0].download, format!("{}/download", server.url));
    let mut wrong_version = server.metadata("2.0.0");
    server.put(
        "/api/fixture/command/universal/1.0.0",
        200,
        serde_json::to_vec(&wrong_version).unwrap(),
    );
    assert!(
        registry
            .search("a&b 😀", &cancel)
            .unwrap_err()
            .to_string()
            .contains("different package version")
    );
    wrong_version["targetPlatform"] = Value::Null;
    server.put(
        "/api/fixture/command/universal/1.0.0",
        200,
        serde_json::to_vec(&wrong_version).unwrap(),
    );
    assert!(registry.search("a&b 😀", &cancel).is_err());
    server.put(path, 200, vec![b' '; 1024 * 1024 + 1]);
    assert!(
        registry
            .search("a&b 😀", &cancel)
            .unwrap_err()
            .to_string()
            .contains("byte budget")
    );
    assert!(registry.search(&"x".repeat(1025), &cancel).is_err());
}

#[test]
fn prerelease_results_do_not_block_stable_search_or_platform_fallback() {
    let server = Server::new();
    let registry = Registry::new(&server.url).unwrap();
    let cancel = AtomicBool::new(false);
    let mut preview = server.metadata("9.0.0");
    preview["name"] = "preview".into();
    preview["preRelease"] = true.into();
    let mut summary = preview.clone();
    summary.as_object_mut().unwrap().remove("targetPlatform");
    summary.as_object_mut().unwrap().remove("preRelease");
    for platform in [target_platform(), "universal"] {
        let mut metadata = preview.clone();
        metadata["targetPlatform"] = platform.into();
        server.put(
            &format!("/api/fixture/preview/{platform}/9.0.0"),
            200,
            serde_json::to_vec(&metadata).unwrap(),
        );
    }
    server.put(
        "/api/-/search?query=rust&size=20",
        200,
        serde_json::to_vec(&json!({"extensions":[
            summary, server.metadata("2.0.0-beta"), preview, server.metadata("1.0.0")
        ]}))
        .unwrap(),
    );
    let results = registry.search("rust", &cancel).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "fixture.command");
    assert_eq!(results[0].version, "1.0.0");

    // A preview-only latest alias is unavailable to stable install/update requests.
    for platform in [target_platform(), "universal"] {
        let mut metadata = server.metadata("1.0.0");
        metadata["targetPlatform"] = platform.into();
        metadata["preRelease"] = true.into();
        server.put(
            &format!("/api/fixture/command/{platform}/latest"),
            200,
            serde_json::to_vec(&metadata).unwrap(),
        );
    }
    assert!(
        registry
            .latest("fixture.command", &cancel)
            .unwrap_err()
            .to_string()
            .contains("No compatible stable package")
    );
    server.latest("1.0.0");
    let stable = registry.latest("fixture.command", &cancel).unwrap();
    assert_eq!(stable.platform, "universal");
    assert_eq!(stable.version, "1.0.0");

    // Filtering must not conceal a registry that returned a different package.
    let mut wrong = server.metadata("1.0.0");
    wrong["name"] = "other".into();
    wrong["preRelease"] = true.into();
    server.put(
        &format!("/api/fixture/command/{}/latest", target_platform()),
        200,
        serde_json::to_vec(&wrong).unwrap(),
    );
    assert!(
        registry
            .latest("fixture.command", &cancel)
            .unwrap_err()
            .to_string()
            .contains("different package identity")
    );
    wrong["name"] = "preview".into();
    wrong["version"] = "8.0.0".into();
    server.put(
        &format!("/api/fixture/preview/{}/9.0.0", target_platform()),
        200,
        serde_json::to_vec(&wrong).unwrap(),
    );
    assert!(
        registry
            .search("rust", &cancel)
            .unwrap_err()
            .to_string()
            .contains("different package version")
    );
}

#[test]
fn cancellation_and_invalid_urls_never_publish_or_touch_network() {
    for url in [
        "file:///tmp/a",
        "http://example.com",
        "https://user:password@example.com",
        "https://example.com/#fragment",
    ] {
        assert!(Registry::new(url).is_err(), "accepted {url}");
    }
    let temp = tempfile::tempdir().unwrap();
    let registry = Registry::new("http://127.0.0.1:1").unwrap();
    let store = Store::new(temp.path().join("store"));
    let cancel = AtomicBool::new(true);
    assert!(
        registry
            .search("test", &cancel)
            .unwrap_err()
            .to_string()
            .contains("canceled")
    );
    let entry = Entry {
        id: "fixture.command".into(),
        version: "1.0.0".into(),
        name: String::new(),
        description: String::new(),
        license: String::new(),
        platform: "universal".into(),
        download: "http://127.0.0.1:1/download".into(),
        checksum: None,
    };
    assert!(registry.install(&entry, &store, &cancel).is_err());
    assert!(!store.root().exists());
}

#[test]
fn fixture_waits_for_fragmented_headers_before_looking_up_a_route() {
    let server = Server::new();
    server.put("/fragment", 200, b"complete".to_vec());
    let mut socket =
        std::net::TcpStream::connect(server.url.trim_start_matches("http://")).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket.write_all(b"GET /frag").unwrap();
    thread::sleep(Duration::from_millis(10));
    socket
        .write_all(b"ment HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("complete"), "{response}");
}
