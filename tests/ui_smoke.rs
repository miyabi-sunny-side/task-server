use axum::{
    body::{Body, Bytes, to_bytes},
    http::{Request, Uri},
};
use html5gum::{DefaultEmitter, Token, Tokenizer};
use hyper_util::rt::TokioIo;
use std::{
    collections::BTreeMap,
    net::{SocketAddr, TcpListener},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tokio::net::TcpStream;

fn referenced_assets(html: &[u8]) -> BTreeMap<String, &'static str> {
    let mut emitter = DefaultEmitter::default();
    emitter.naively_switch_states(true); // Keep inline script contents out of the tag stream.
    let mut assets = BTreeMap::new();
    for token in Tokenizer::new_with_emitter(html, emitter) {
        let Token::StartTag(tag) = token.unwrap() else {
            continue;
        };
        let attribute = |name: &[u8]| tag.attributes.get(name).map(|value| value.as_slice());
        let (path, mime) = match tag.name.as_slice() {
            b"script" => (attribute(b"src"), "text/javascript"),
            b"link" => {
                let mime = match attribute(b"rel") {
                    Some(b"stylesheet") => "text/css",
                    Some(b"manifest") => "application/manifest+json",
                    Some(b"icon") => "image/svg+xml",
                    Some(b"apple-touch-icon") => "image/png",
                    _ => continue,
                };
                (
                    Some(attribute(b"href").expect("asset link needs href")),
                    mime,
                )
            }
            _ => continue,
        };
        if let Some(path) = path {
            assets.insert(String::from_utf8(path.to_vec()).unwrap(), mime);
        }
    }
    assets
}

#[test]
fn referenced_assets_follow_tags_and_attributes() {
    let html = br#"<!doctype html>
      <!-- <script src='/comment.js'></script> -->
      <script>const decoy = "<link rel='icon' href='/inline.svg'>";</script>
      <script defer src='/app.js?x=1&amp;y=2'></script>
      <link href="/app.css" crossorigin rel="stylesheet">
      <link rel=manifest href='/install.webmanifest'>
      <link rel=icon href='/icon.svg'>
      <link rel=apple-touch-icon href='/apple.png'>
      <link rel=preload href='/unused.js'>"#;
    assert_eq!(
        referenced_assets(html),
        BTreeMap::from([
            ("/app.js?x=1&y=2".into(), "text/javascript"),
            ("/app.css".into(), "text/css"),
            ("/install.webmanifest".into(), "application/manifest+json"),
            ("/icon.svg".into(), "image/svg+xml"),
            ("/apple.png".into(), "image/png"),
        ])
    );
}

async fn fetch(address: SocketAddr, method: &str, path: &str, status: u16) -> (String, Bytes) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let stream = TcpStream::connect(address).await.unwrap();
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .unwrap();
        let connection = tokio::spawn(connection);
        let response = sender
            .send_request(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("Host", address.to_string())
                    .header("Connection", "close")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status, "{method} {path}");
        let mime = response
            .headers()
            .get("content-type")
            .map_or("", |value| value.to_str().unwrap())
            .split(';')
            .next()
            .unwrap()
            .trim()
            .to_ascii_lowercase();
        let body = to_bytes(Body::new(response.into_body()), usize::MAX)
            .await
            .unwrap();
        drop(sender);
        connection.await.unwrap().unwrap();
        (mime, body)
    })
    .await
    .expect("HTTP request or body timed out")
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn packaged_ui() {
    let root = tempfile::tempdir().unwrap();
    let mut server = None;
    let address = if let Ok(base) = std::env::var("TASK_SERVER_SMOKE_URL") {
        let uri: Uri = base.parse().unwrap();
        assert_eq!(uri.scheme_str(), Some("http"));
        uri.authority()
            .unwrap()
            .as_str()
            .parse::<SocketAddr>()
            .unwrap()
    } else {
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap();
        drop(reservation);
        let binary = root
            .path()
            .join(format!("task-server{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(env!("CARGO_BIN_EXE_task-server"), &binary).unwrap();
        server = Some(Server(
            Command::new(binary)
                .env_clear()
                .current_dir(root.path())
                .env("PORT", address.port().to_string())
                .env("APP_BIND_ADDR", "invalid-legacy-address")
                .env("LOG_LEVEL", "warn")
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        ));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(
                server.as_mut().unwrap().0.try_wait().unwrap().is_none(),
                "server exited early"
            );
            if TcpStream::connect(address).await.is_ok() {
                break;
            }
            assert!(Instant::now() < deadline, "server did not start");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(root.path().join("data/ledger/tasks").is_dir());
        assert!(!root.path().join("client").exists());
        address
    };
    assert!(
        address.ip().is_loopback(),
        "smoke checks need an isolated loopback server"
    );
    check_ui(address).await;
    drop(server);
}

async fn check_ui(address: SocketAddr) {
    assert_eq!(fetch(address, "GET", "/healthz", 200).await.1, "ok\n");
    let (mime, health) = fetch(address, "GET", "/api/health", 200).await;
    assert_eq!(mime, "application/json");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&health).unwrap(),
        serde_json::json!({"status":"ok"})
    );
    let (mime, html) = fetch(address, "GET", "/", 200).await;
    assert_eq!(mime, "text/html");
    assert!(html.to_ascii_lowercase().starts_with(b"<!doctype html>"));
    let assets = referenced_assets(&html);
    for mime in [
        "text/javascript",
        "text/css",
        "application/manifest+json",
        "image/svg+xml",
        "image/png",
    ] {
        assert!(
            assets.values().any(|value| *value == mime),
            "missing {mime}"
        );
    }
    for (path, mime) in assets {
        let (actual, body) = fetch(address, "GET", &path, 200).await;
        assert_eq!(actual, mime, "{path}");
        assert!(!body.is_empty(), "{path}");
        let (head_mime, head_body) = fetch(address, "HEAD", &path, 200).await;
        assert_eq!(head_mime, mime, "{path}");
        assert!(head_body.is_empty(), "{path}");
        if mime == "application/manifest+json" {
            let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
            for icon in manifest["icons"].as_array().unwrap() {
                let (icon_mime, icon_body) =
                    fetch(address, "GET", icon["src"].as_str().unwrap(), 200).await;
                assert_eq!(icon_mime, icon["type"].as_str().unwrap());
                assert!(icon_body.starts_with(b"\x89PNG\r\n\x1a\n"));
            }
        }
    }
    for path in [
        "/tasks/example",
        "/closed",
        "/products",
        "/projects/example",
    ] {
        assert_eq!(
            fetch(address, "GET", path, 200).await,
            ("text/html".into(), html.clone()),
            "{path}"
        );
    }
    for path in ["/api", "/api/", "/api/missing"] {
        assert_eq!(
            fetch(address, "GET", path, 404).await.0,
            "application/json",
            "{path}"
        );
    }
    fetch(address, "POST", "/", 405).await;
    let (_, snapshot) = fetch(address, "GET", "/worker/snapshot", 200).await;
    assert!(
        serde_json::from_slice::<serde_json::Value>(&snapshot)
            .unwrap()
            .get("tasks")
            .is_some()
    );
}
