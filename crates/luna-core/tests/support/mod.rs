//! A minimal HTTP/1.1 server for download tests. It speaks just enough HTTP, over raw TCP, to let a
//! test decide exactly what happens: ignore `Range`, drop the connection mid-body, stall, answer 503.

// Shared by several test crates, and each one uses only part of it.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Debug, Clone)]
pub struct Request {
    pub path: String,
    /// The raw `Range` header, e.g. `bytes=100-`.
    pub range: Option<String>,
}

impl Request {
    /// The start offset of a `bytes=N-` range.
    pub fn range_start(&self) -> Option<u64> {
        self.range
            .as_deref()?
            .strip_prefix("bytes=")?
            .split('-')
            .next()?
            .parse()
            .ok()
    }
}

type Handler = dyn Fn(&Request, usize, &mut TcpStream) + Send + Sync;

pub struct Server {
    addr: SocketAddr,
    log: Arc<Mutex<Vec<Request>>>,
}

impl Server {
    /// Start a server on a free loopback port. `handler` gets the request, the zero-based number of
    /// this request across the whole server, and the socket to answer on.
    pub fn start(
        handler: impl Fn(&Request, usize, &mut TcpStream) + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let log: Arc<Mutex<Vec<Request>>> = Arc::default();
        let handler: Arc<Handler> = Arc::new(handler);
        let counter = Arc::new(AtomicUsize::new(0));

        let accept_log = Arc::clone(&log);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (handler, log, counter) = (
                    Arc::clone(&handler),
                    Arc::clone(&accept_log),
                    Arc::clone(&counter),
                );
                thread::spawn(move || serve_connection(stream, &*handler, &log, &counter));
            }
        });
        Self { addr, log }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().unwrap().clone()
    }

    pub fn request_count(&self) -> usize {
        self.log.lock().unwrap().len()
    }
}

fn serve_connection(
    mut stream: TcpStream,
    handler: &Handler,
    log: &Mutex<Vec<Request>>,
    counter: &AtomicUsize,
) {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
        if head.len() > 16 * 1024 {
            return;
        }
    }
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.lines();
    let path = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_owned();
    let range = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("range"))
        .map(|(_, value)| value.trim().to_owned());
    let request = Request { path, range };

    log.lock().unwrap().push(request.clone());
    let nth = counter.fetch_add(1, Ordering::SeqCst);
    handler(&request, nth, &mut stream);
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Both);
}

/// Write a complete response.
pub fn respond(stream: &mut TcpStream, status: &str, headers: &[(&str, String)], body: &[u8]) {
    let mut out = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.write_all(body);
}

/// Serve `data` honouring `Range: bytes=N-` like a normal file server.
pub fn serve(stream: &mut TcpStream, request: &Request, data: &[u8]) {
    let len = data.len() as u64;
    match request.range_start() {
        Some(start) if start < len => {
            let headers = [("Content-Range", format!("bytes {start}-{}/{len}", len - 1))];
            respond(
                stream,
                "206 Partial Content",
                &headers,
                &data[start as usize..],
            );
        }
        Some(_) => {
            respond(
                stream,
                "416 Range Not Satisfiable",
                &[("Content-Range", format!("bytes */{len}"))],
                b"",
            );
        }
        None => respond(stream, "200 OK", &[], data),
    }
}

/// Serve the whole file with 200, ignoring any `Range` header.
pub fn serve_ignoring_range(stream: &mut TcpStream, data: &[u8]) {
    respond(stream, "200 OK", &[], data);
}

/// Promise the whole body, send only `data[from..from + send]`, then return so the caller closes the
/// connection. If `stall` is set, wait that long first, simulating a server that went quiet.
pub fn send_part_of(
    stream: &mut TcpStream,
    data: &[u8],
    from: usize,
    send: usize,
    stall: Option<std::time::Duration>,
) {
    let head = format!(
        "HTTP/1.1 {}\r\nConnection: close\r\nContent-Length: {}\r\n{}\r\n",
        if from == 0 {
            "200 OK"
        } else {
            "206 Partial Content"
        },
        data.len() - from,
        if from == 0 {
            String::new()
        } else {
            format!(
                "Content-Range: bytes {from}-{}/{}\r\n",
                data.len() - 1,
                data.len()
            )
        },
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&data[from..from + send]);
    let _ = stream.flush();
    if let Some(wait) = stall {
        thread::sleep(wait);
    }
}

/// Deterministic, non-repeating-looking bytes, so a mix-up of offsets changes the checksum.
pub fn sample_body(len: usize) -> Vec<u8> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}
