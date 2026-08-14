use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;

use crate::error::Error;
use crate::outbox::NotificationIntent;

#[derive(Debug, thiserror::Error)]
#[error("notify failed: {0}")]
pub struct NotifyError(pub String);

pub trait Notifier: Send + Sync {
    /// Best-effort adapter. Failure must not roll back a status commit.
    ///
    /// # Errors
    ///
    /// Returns `NotifyError` when the adapter cannot deliver.
    fn notify(&self, intent: &NotificationIntent) -> Result<(), NotifyError>;
}

pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn notify(&self, _intent: &NotificationIntent) -> Result<(), NotifyError> {
        Ok(())
    }
}

pub struct FailingNotifier;

impl Notifier for FailingNotifier {
    fn notify(&self, _intent: &NotificationIntent) -> Result<(), NotifyError> {
        Err(NotifyError("forced failure".into()))
    }
}

/// POST JSON to an HTTP URL (ntfy-compatible: `http://host:port/topic`).
pub struct HttpNotifier {
    pub url: String,
}

impl Notifier for HttpNotifier {
    fn notify(&self, intent: &NotificationIntent) -> Result<(), NotifyError> {
        let body = serde_json::to_vec(intent).map_err(|err| NotifyError(err.to_string()))?;
        let status = post_json(&self.url, &body)?;
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(NotifyError(format!("http {status}")))
        }
    }
}

fn post_json(url: &str, body: &[u8]) -> Result<u16, NotifyError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| NotifyError("NTFY_URL must be http://".into()))?;
    let (hostport, path) = match rest.split_once('/') {
        Some((host, tail)) => (host, format!("/{tail}")),
        None => (rest, "/".to_owned()),
    };
    let (host, port) = if let Some((host, port)) = hostport.split_once(':') {
        (
            host,
            port.parse::<u16>()
                .map_err(|err| NotifyError(format!("bad port: {err}")))?,
        )
    } else {
        (hostport, 80)
    };
    let addr = format!("{host}:{port}");
    let mut stream =
        TcpStream::connect(&addr).map_err(|err| NotifyError(format!("connect {addr}: {err}")))?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {hostport}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.write_all(body))
        .map_err(|err| NotifyError(err.to_string()))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|err| NotifyError(err.to_string()))?;
    let status_line = response.lines().next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| NotifyError(format!("bad status line: {status_line}")))?;
    Ok(status)
}

/// Deliver pending outbox files. Success (`Ok` from the adapter, i.e. HTTP 2xx
/// for `HttpNotifier`) atomically rewrites `state` to `delivered`.
pub fn flush_pending(outbox_dir: &Path, notifier: &dyn Notifier) -> Result<usize, Error> {
    if !outbox_dir.is_dir() {
        return Ok(0);
    }
    let mut delivered = 0;
    let mut entries: Vec<_> = fs::read_dir(outbox_dir)?.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read(&path)?;
        let mut intent: NotificationIntent = serde_json::from_slice(&raw)?;
        if intent.state != "pending" {
            continue;
        }
        if notifier.notify(&intent).is_err() {
            continue;
        }
        intent.state = "delivered".into();
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&intent)?)?;
        fs::rename(tmp, path)?;
        delivered += 1;
    }
    Ok(delivered)
}
