//! Utilidades compartidas por las pruebas.

use std::io::{Read, Write};
use std::thread;

use serde_json::Value;

/// Lo que recibió el servidor falso: método, ruta, cabecera `Authorization` y cuerpo.
pub type Request = (String, String, Option<String>, String);

/// Servidor HTTP mínimo: responde según `(método, ruta)` y guarda lo que recibió.
/// `routes(method, path, auth, body) -> (status, json)`.
pub fn fake_http(
    routes: impl Fn(&str, &str, Option<&str>, &str) -> (u16, Value) + Send + 'static,
    requests: usize,
) -> (u16, thread::JoinHandle<Vec<Request>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let mut seen = Vec::new();
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            let (head_end, content_len) = loop {
                let n = stream.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..i]).to_ascii_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0usize);
                    break (i + 4, len);
                }
            };
            while buf.len() < head_end + content_len {
                let n = stream.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
            }
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
            let body_text = body.clone();
            let mut first = head.lines().next().unwrap().split(' ');
            let (method, path) = (
                first.next().unwrap().to_string(),
                first.next().unwrap().to_string(),
            );
            let auth = head
                .lines()
                .find_map(|l| l.strip_prefix("Authorization: ").map(str::to_string));
            let (status, payload) = routes(&method, &path, auth.as_deref(), &body);
            let body = payload.to_string();
            write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            seen.push((method, path, auth, body_text));
        }
        seen
    });
    (port, handle)
}
