//! Local HTTP fixture servers.
//!
//! Every conformance test in this crate runs against one of these. Nothing here
//! reaches a real provider: live checks are explicitly `#[ignore]`d and need an
//! environment variable, so normal test runs need no network and no paid key.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// What the fixture server sends back.
pub(crate) enum Reply {
    /// A complete response.
    Body {
        status_line: &'static str,
        content_type: &'static str,
        body: String,
    },
    /// A cross-origin redirect, to prove credentials are not replayed.
    #[cfg(feature = "remote")]
    Redirect { location: String },
    /// Headers promising more bytes than the body carries, then a close.
    Truncated {
        declared_length: usize,
        body: String,
    },
    /// Accept, read the request, and never answer.
    Silence,
}

/// What the server saw.
pub(crate) struct Captured {
    pub(crate) request: String,
}

impl Captured {
    pub(crate) fn contains(&self, needle: &str) -> bool {
        self.request.contains(needle)
    }
}

/// Starts a one-shot server and returns its base URL and a handle to what it
/// captured.
pub(crate) fn serve(reply: Reply) -> (String, JoinHandle<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let request = read_request(&mut stream);
        match reply {
            Reply::Body {
                status_line,
                content_type,
                body,
            } => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
            }
            #[cfg(feature = "remote")]
            Reply::Redirect { location } => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
            Reply::Truncated {
                declared_length,
                body,
            } => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {declared_length}\r\nConnection: close\r\n\r\n{body}",
                );
            }
            Reply::Silence => {
                thread::sleep(Duration::from_millis(750));
            }
        }
        Captured { request }
    });
    (format!("http://{address}/"), server)
}

/// Starts a server that answers nothing and reports whether anyone connected
/// within `window`.
pub(crate) fn serve_unvisited(window: Duration) -> (String, JoinHandle<Option<Captured>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_millis(200)))
                        .expect("read timeout");
                    return Some(Captured {
                        request: read_request(&mut stream),
                    });
                }
                Err(_) => thread::sleep(Duration::from_millis(10)),
            }
        }
        None
    });
    (format!("http://{address}/"), server)
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut buffer = [0u8; 16 * 1024];
    let read = stream.read(&mut buffer).unwrap_or_default();
    String::from_utf8_lossy(&buffer[..read]).into_owned()
}

/// A 200 response carrying `body`.
pub(crate) fn ok(content_type: &'static str, body: String) -> Reply {
    Reply::Body {
        status_line: "200 OK",
        content_type,
        body,
    }
}

/// A response with an explicit status line.
pub(crate) fn status(status_line: &'static str, body: String) -> Reply {
    Reply::Body {
        status_line,
        content_type: "application/json",
        body,
    }
}
