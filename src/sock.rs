//! The pane's control socket: `pane.sock` in the root's state directory.
//! One JSON request per connection, one JSON reply, newline terminated.
//! The one-shot subcommands connect, send, read, and exit.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "pane.sock";

const IO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    Run,
    RunFailed,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

impl Response {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            status: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            status: None,
        }
    }
}

/// What the pane reports for `status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub root: String,
    pub running: bool,
    pub queued: bool,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub build_errors: u32,
    pub auto_run: bool,
}

/// Whether a pane is answering at `path`. A `status` round trip, not a bare
/// `connect`: on macOS `connect` to a path whose listener has closed can
/// succeed under load, and the stream then reads EOF at once.
pub fn probe(path: &Path) -> bool {
    send(path, &Request::Status).is_ok()
}

/// Sends one request and returns the reply.
pub fn send(path: &Path, req: &Request) -> Result<Response, String> {
    let mut stream =
        UnixStream::connect(path).map_err(|e| format!("connect {}: {e}", path.display()))?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok();
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok();
    let mut line = serde_json::to_string(req).map_err(|e| e.to_string())?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    let mut reply = String::new();
    BufReader::new(&stream)
        .read_line(&mut reply)
        .map_err(|e| format!("read: {e}"))?;
    if reply.trim().is_empty() {
        return Err("pane closed the connection without replying".into());
    }
    serde_json::from_str(&reply).map_err(|e| format!("reply: {e}"))
}

/// Binds `path`. A stale socket file is removed; a live one means another
/// pane owns this root.
pub fn bind(path: &Path) -> Result<UnixListener, String> {
    if path.exists() {
        if probe(path) {
            return Err(format!(
                "another Tests pane is already open for this project ({})",
                path.display()
            ));
        }
        std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    UnixListener::bind(path).map_err(|e| format!("bind {}: {e}", path.display()))
}

/// Serves `listener` on its own thread. `handle` runs per request and its
/// return is the reply; a request that does not parse gets an error reply.
pub fn serve(listener: UnixListener, handle: impl Fn(Request) -> Response + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            stream.set_read_timeout(Some(IO_TIMEOUT)).ok();
            stream.set_write_timeout(Some(IO_TIMEOUT)).ok();
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let reply = match serde_json::from_str::<Request>(&line) {
                Ok(req) => handle(req),
                Err(e) => Response::err(format!("bad request: {e}")),
            };
            let mut text = serde_json::to_string(&reply).unwrap_or_default();
            text.push('\n');
            let _ = stream.write_all(text.as_bytes());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_wire_format() {
        assert_eq!(
            serde_json::to_string(&Request::RunFailed).unwrap(),
            r#"{"cmd":"run-failed"}"#
        );
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"cmd":"status"}"#).unwrap(),
            Request::Status
        );
    }

    #[test]
    fn round_trip_over_a_socket() {
        let path = crate::testutil::tempdir("sock").join(FILE);
        assert!(!probe(&path));
        let listener = bind(&path).unwrap();
        serve(listener, |req| match req {
            Request::Run => Response::ok("queued"),
            Request::RunFailed => Response::err("nothing failed"),
            Request::Status => Response {
                ok: true,
                message: String::new(),
                status: Some(Status {
                    root: "/p".into(),
                    failed: 2,
                    ..Default::default()
                }),
            },
        });
        assert!(probe(&path));
        assert_eq!(send(&path, &Request::Run).unwrap(), Response::ok("queued"));
        assert_eq!(
            send(&path, &Request::RunFailed).unwrap(),
            Response::err("nothing failed")
        );
        let s = send(&path, &Request::Status).unwrap().status.unwrap();
        assert_eq!((s.root.as_str(), s.failed), ("/p", 2));
        // A second bind on a live socket is refused.
        assert!(bind(&path).is_err());
    }

    #[test]
    fn stale_socket_file_is_replaced() {
        let path = crate::testutil::tempdir("sock-stale").join(FILE);
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists(), "{} vanished", path.display());
        assert!(!probe(&path), "{} still answers", path.display());
        assert!(bind(&path).is_ok());
    }
}
