use crate::control::Control;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TlsOptions {
    pub enabled: bool,
    pub cert: Option<String>,
    pub cert_key: Option<String>,
    pub protocols: Option<String>,
    pub ciphers: Option<String>,
    pub ciphersuites: Option<String>,
    pub dhparam: Option<String>,
}
impl TlsOptions {
    pub fn acceptor(&self) -> Result<Option<openssl::ssl::SslAcceptor>> {
        use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod, SslVersion};
        if !self.enabled {
            return Ok(None);
        }
        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls_server())?;
        builder.set_min_proto_version(Some(SslVersion::TLS1_2))?;
        if let Some(protocols) = &self.protocols {
            let versions: Vec<_> = protocols.split_whitespace().collect();
            anyhow::ensure!(
                !versions.is_empty()
                    && versions.iter().all(|v| matches!(*v, "TLSv1.2" | "TLSv1.3")),
                "API TLS protocols must be TLSv1.2 and/or TLSv1.3"
            );
            if !versions.contains(&"TLSv1.2") {
                builder.set_min_proto_version(Some(SslVersion::TLS1_3))?;
            }
            if !versions.contains(&"TLSv1.3") {
                builder.set_max_proto_version(Some(SslVersion::TLS1_2))?;
            }
        }
        builder
            .set_certificate_chain_file(self.cert.as_deref().context("tls.cert is required")?)?;
        builder.set_private_key_file(
            self.cert_key
                .as_deref()
                .context("tls.cert_key is required")?,
            SslFiletype::PEM,
        )?;
        builder.check_private_key()?;
        if let Some(ciphers) = &self.ciphers {
            builder.set_cipher_list(ciphers)?;
        }
        if let Some(suites) = &self.ciphersuites {
            builder.set_ciphersuites(suites)?;
        }
        if let Some(path) = &self.dhparam {
            let params = openssl::dh::Dh::params_from_pem(&std::fs::read(path)?)?;
            builder.set_tmp_dh(&params)?;
        }
        Ok(Some(builder.build()))
    }
}

pub type Handler = Arc<dyn Fn(&str) -> (u16, Value) + Send + Sync>;
pub type RequestHandler = Arc<dyn Fn(&str, &str, &[u8]) -> (u16, Value) + Send + Sync>;
pub struct Server {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub address: std::net::SocketAddr,
}

impl Server {
    pub fn start(host: &str, port: u16, handler: Handler, control: Control) -> Result<Self> {
        Self::start_authenticated(host, port, None, handler, control)
    }

    pub fn start_authenticated(
        host: &str,
        port: u16,
        token: Option<String>,
        handler: Handler,
        control: Control,
    ) -> Result<Self> {
        Self::start_requests(
            host,
            port,
            token,
            Arc::new(move |method, path, _| {
                if method == "GET" {
                    handler(path)
                } else {
                    (405, json!({"error":"method not allowed"}))
                }
            }),
            control,
        )
    }

    pub fn start_requests(
        host: &str,
        port: u16,
        token: Option<String>,
        handler: RequestHandler,
        control: Control,
    ) -> Result<Self> {
        Self::start_tls(host, port, token, None, handler, control)
    }

    pub fn start_tls(
        host: &str,
        port: u16,
        token: Option<String>,
        acceptor: Option<openssl::ssl::SslAcceptor>,
        handler: RequestHandler,
        control: Control,
    ) -> Result<Self> {
        let scheme = if acceptor.is_some() { "https" } else { "http" };
        let listener = TcpListener::bind((host, port)).context("bind HTTP API")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("http-api".into())
            .spawn(move || {
                let _panic = control.watch_thread("HTTP worker");
                while !thread_stop.load(Ordering::Relaxed) && !control.stopped() {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            if stream
                                .set_read_timeout(Some(Duration::from_millis(200)))
                                .is_err()
                                || stream
                                    .set_write_timeout(Some(Duration::from_millis(500)))
                                    .is_err()
                            {
                                continue;
                            }
                            if let Some(acceptor) = &acceptor {
                                if let Ok(mut tls) = acceptor.accept(stream) {
                                    let _ =
                                        serve(&mut tls, &handler, &thread_stop, token.as_deref());
                                }
                            } else {
                                let _ =
                                    serve(&mut stream, &handler, &thread_stop, token.as_deref());
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20))
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            control.fail(format!("HTTP accept failed: {error}"));
                            break;
                        }
                    }
                }
            })
            .context("spawn HTTP thread")?;
        crate::status!("[http] listening on {scheme}://{address}");
        Ok(Self {
            stop,
            thread: Some(thread),
            address,
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    stream: &mut (impl Read + Write),
    handler: &RequestHandler,
    stop: &AtomicBool,
    token: Option<&str>,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut request = Vec::new();
    let mut chunk = [0; 2048];
    while !request.windows(4).any(|part| part == b"\r\n\r\n") {
        if stop.load(Ordering::Relaxed) || Instant::now() >= deadline || request.len() >= 8192 {
            return Ok(());
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(size) => request.extend_from_slice(&chunk[..size]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue
            }
            Err(error) => return Err(error.into()),
        }
    }
    let header_end = request
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap()
        + 4;
    anyhow::ensure!(header_end <= 8192, "HTTP headers too large");
    let text = std::str::from_utf8(&request[..header_end]).context("invalid HTTP request")?;
    let mut words = text.lines().next().unwrap_or_default().split_whitespace();
    let method = words.next().unwrap_or_default();
    let path = words.next().unwrap_or("/").split('?').next().unwrap_or("/");
    let authorized = token.is_none_or(|expected| {
        let mut headers = text
            .lines()
            .skip(1)
            .take_while(|line| !line.is_empty())
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("authorization")
                    .then_some(value.trim())
            });
        let valid = headers
            .next()
            .and_then(|value| value.split_once(' '))
            .is_some_and(|(scheme, value)| {
                scheme.eq_ignore_ascii_case("bearer")
                    && value.len() == expected.len()
                    && openssl::memcmp::eq(value.as_bytes(), expected.as_bytes())
            });
        valid && headers.next().is_none()
    });
    let method = method.to_owned();
    let path = path.to_owned();
    let mut lengths = Vec::new();
    let mut transfer_encoding = false;
    for line in text.lines().skip(1).take_while(|line| !line.is_empty()) {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                lengths.push(value.trim().parse::<usize>());
            }
            if name.eq_ignore_ascii_case("transfer-encoding") {
                transfer_encoding = true;
            }
        }
    }
    let invalid = transfer_encoding || lengths.len() > 1 || lengths.iter().any(Result::is_err);
    let length = lengths
        .first()
        .and_then(|n| n.as_ref().ok())
        .copied()
        .unwrap_or(0);
    // Drain a bounded, well-framed body even when authentication fails. This
    // avoids closing TLS while the client is still writing its request body.
    // Authentication still precedes dispatch and no untrusted body is parsed.
    if !authorized && !invalid && length <= 65536 {
        while request.len() < header_end + length
            && Instant::now() < deadline
            && !stop.load(Ordering::Relaxed)
        {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(size) => request.extend_from_slice(&chunk[..size]),
            }
        }
    }
    let (status, body) = if !authorized {
        (401, json!({"error":"unauthorized"}))
    } else if invalid {
        (400, json!({"error":"invalid request framing"}))
    } else if length > 65536 {
        (413, json!({"error":"request body too large"}))
    } else {
        while request.len() < header_end + length {
            if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
                return Ok(());
            }
            match stream.read(&mut chunk) {
                Ok(0) => return Ok(()),
                Ok(size) => request.extend_from_slice(&chunk[..size]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    continue
                }
                Err(e) => return Err(e.into()),
            }
        }
        handler(&method, &path, &request[header_end..header_end + length])
    };
    let body = body.to_string();
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}", body.len())?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;
    #[test]
    fn controls_require_authentication_and_unambiguous_complete_bodies() {
        use std::sync::atomic::AtomicUsize;
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let server = Server::start_requests(
            "127.0.0.1",
            0,
            Some("token".into()),
            Arc::new(move |method, path, body| {
                assert_eq!((method, path), ("POST", "/json_rpc"));
                assert_eq!(body, br#"{"method":"pause"}"#);
                count.fetch_add(1, Ordering::Relaxed);
                (200, json!({"ok":true}))
            }),
            Control::new(Arc::new(AtomicBool::new(false))),
        )
        .unwrap();
        for (auth, framing, expected) in [
            ("", "Content-Length: 18\r\n", 401),
            (
                "Authorization: Bearer token\r\n",
                "Content-Length: 18\r\nContent-Length: 18\r\n",
                400,
            ),
            (
                "Authorization: Bearer token\r\n",
                "Transfer-Encoding: chunked\r\n",
                400,
            ),
            (
                "Authorization: Bearer token\r\n",
                "Content-Length: 999999\r\n",
                413,
            ),
            (
                "Authorization: Bearer token\r\n",
                "Content-Length: 18\r\n",
                200,
            ),
        ] {
            let mut stream = TcpStream::connect(server.address).unwrap();
            write!(
                stream,
                "POST /json_rpc HTTP/1.1\r\nHost: localhost\r\n{auth}{framing}\r\n"
            )
            .unwrap();
            if expected == 200 {
                stream.write_all(br#"{"method":"pause"}"#).unwrap();
            }
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(
                response.starts_with(&format!("HTTP/1.1 {expected} ")),
                "{response}"
            );
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn bearer_token_protects_every_endpoint() {
        let server = Server::start_authenticated(
            "127.0.0.1",
            0,
            Some("local-test-token".into()),
            Arc::new(|_| (200, json!({"ok":true}))),
            Control::new(Arc::new(AtomicBool::new(false))),
        )
        .unwrap();
        for (headers, expected) in [
            ("", 401),
            ("Authorization: Bearer wrong\r\n", 401),
            ("Authorization: Basic local-test-token\r\n", 401),
            (
                "Authorization: Bearer local-test-token\r\nAuthorization: Bearer wrong\r\n",
                401,
            ),
            ("authorization: Bearer local-test-token\r\n", 200),
        ] {
            let mut stream = TcpStream::connect(server.address).unwrap();
            write!(
                stream,
                "GET /summary HTTP/1.1\r\nHost: localhost\r\n{headers}\r\n"
            )
            .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(
                response.starts_with(&format!("HTTP/1.1 {expected} ")),
                "{response}"
            );
        }
    }

    #[test]
    fn handler_panic_stops_coordinator() {
        let control = Control::new(Arc::new(AtomicBool::new(false)));
        let server = Server::start(
            "127.0.0.1",
            0,
            Arc::new(|_| panic!("injected handler failure")),
            control.clone(),
        )
        .unwrap();
        let mut stream = TcpStream::connect(server.address).unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !control.stopped() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(control.stopped());
        assert_eq!(control.error().as_deref(), Some("HTTP worker panicked"));
        drop(server);
    }

    #[test]
    fn fragmented_requests_and_idle_shutdown() {
        let server = Server::start(
            "127.0.0.1",
            0,
            Arc::new(|path| (200, json!({"path":path}))),
            Control::new(Arc::new(AtomicBool::new(false))),
        )
        .unwrap();
        let mut stream = TcpStream::connect(server.address).unwrap();
        stream.write_all(b"GET /summary?x=1 HTTP/1.1\r\n").unwrap();
        thread::sleep(Duration::from_millis(30));
        stream.write_all(b"Host: localhost\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains(r#"{"path":"/summary"}"#));
        let _idle = TcpStream::connect(server.address).unwrap();
        thread::sleep(Duration::from_millis(30));
        let start = Instant::now();
        drop(server);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
