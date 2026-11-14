use crate::support::{wait_until, Process};
use anyhow::{bail, ensure, Context, Result};
use rustls::{
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct Certificate {
    pub cert: PathBuf,
    pub key: PathBuf,
    pub fingerprint: String,
}
impl Certificate {
    pub fn new(directory: &Path) -> Result<Self> {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])?;
        let result = Self {
            cert: directory.join("pool.crt"),
            key: directory.join("pool.key"),
            fingerprint: hex::encode(Sha256::digest(cert.der().as_ref())),
        };
        fs::write(&result.cert, cert.pem())?;
        fs::write(&result.key, signing_key.serialize_pem())?;
        fs::set_permissions(&result.key, fs::Permissions::from_mode(0o600))?;
        Ok(result)
    }

    fn acceptor(&self) -> Result<Arc<ServerConfig>> {
        Ok(Arc::new(
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()?
                .with_no_client_auth()
                .with_single_cert(
                    CertificateDer::pem_file_iter(&self.cert)?
                        .collect::<std::result::Result<Vec<_>, _>>()?,
                    PrivateKeyDer::from_pem_file(&self.key)?,
                )?,
        ))
    }
}

pub trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

#[derive(Default)]
struct Connections {
    stop: AtomicBool,
    sockets: Mutex<Vec<TcpStream>>,
}
impl Connections {
    fn register(&self, socket: &TcpStream) -> Result<()> {
        socket.set_nonblocking(false)?;
        socket.set_read_timeout(Some(Duration::from_secs(60)))?;
        socket.set_write_timeout(Some(Duration::from_secs(10)))?;
        let mut sockets = self.sockets.lock().unwrap();
        if self.stop.load(Ordering::Acquire) {
            socket.shutdown(Shutdown::Both)?;
        } else {
            sockets.push(socket.try_clone()?);
        }
        Ok(())
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        for socket in self.sockets.lock().unwrap().iter() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}

struct CloseSocket(TcpStream);
impl Drop for CloseSocket {
    fn drop(&mut self) {
        // The cancellation registry owns another descriptor. Explicit shutdown
        // makes EOF visible to the miner as soon as this handler returns.
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

// The listener is bound before its address is returned. Every connection is
// registered for cancellation, and every fixture thread is joined on drop.
struct Server {
    port: u16,
    connections: Arc<Connections>,
    thread: Option<JoinHandle<()>>,
}
impl Server {
    fn new(
        handler: impl Fn(TcpStream, &Connections) -> Result<()> + Send + Sync + 'static,
    ) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let connections = Arc::new(Connections::default());
        let state = connections.clone();
        let handler = Arc::new(handler);
        let thread = thread::spawn(move || {
            let mut workers = Vec::new();
            while !state.stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        if state.register(&socket).is_err() {
                            continue;
                        }
                        let state = state.clone();
                        let handler = handler.clone();
                        workers.push(thread::spawn(move || {
                            let _close =
                                CloseSocket(socket.try_clone().expect("clone fixture socket"));
                            if let Err(error) = handler(socket, &state) {
                                if !state.stop.load(Ordering::Acquire) {
                                    eprintln!("Local fixture connection: {error:#}");
                                }
                            }
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => {
                        eprintln!("Local listener: {error}");
                        break;
                    }
                }
            }
            state.stop();
            let failed = workers
                .into_iter()
                .map(|worker| worker.join().is_err())
                .fold(false, |any, failed| any | failed);
            assert!(!failed, "local fixture worker panicked");
        });
        Ok(Self {
            port,
            connections,
            thread: Some(thread),
        })
    }

    fn stop(&mut self) {
        self.connections.stop();
        if let Some(thread) = self.thread.take() {
            let failed = thread.join().is_err();
            if !thread::panicking() {
                assert!(!failed, "local fixture listener panicked");
            }
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Default)]
pub struct PoolState {
    pub shares: Vec<(String, String)>,
    pub agents: Vec<String>,
    pub keepalives: usize,
}

pub struct Pool {
    server: Server,
    state: Arc<Mutex<PoolState>>,
    available: Arc<AtomicBool>,
}
impl Pool {
    pub fn new(certificate: Option<&Certificate>, deny_dev: bool, nonce: u32) -> Result<Self> {
        let acceptor = certificate.map(Certificate::acceptor).transpose()?;
        let state = Arc::new(Mutex::new(PoolState::default()));
        let shared = state.clone();
        let available = Arc::new(AtomicBool::new(true));
        let serving = available.clone();
        let server = Server::new(move |socket, _| {
            if !serving.load(Ordering::Acquire) {
                return Ok(());
            }
            let connection: Box<dyn Stream> = match &acceptor {
                Some(tls) => Box::new(StreamOwned::new(
                    ServerConnection::new(tls.clone())?,
                    socket,
                )),
                None => Box::new(socket),
            };
            let mut connection = BufReader::new(connection);
            let mut blob = [0x42; 76];
            blob[0] = 0x0c;
            blob[39..43].copy_from_slice(&nonce.to_le_bytes());
            let job = json!({"blob": hex::encode(blob), "job_id": "rxminer-test-job",
                "target": "ffffffffffffffff", "height": 3500000,
                "seed_hash": hex::encode([0u8; 32]), "algo": "rx/0"});
            let mut wallet = String::new();
            loop {
                let mut line = String::new();
                if connection.read_line(&mut line)? == 0 {
                    break;
                }
                ensure!(line.len() < 65536, "pool request too large");
                let request: Value = serde_json::from_str(&line)?;
                let params = &request["params"];
                let mut error = Value::Null;
                let result = match request["method"].as_str() {
                    Some("login") => {
                        wallet = params["login"].as_str().context("login wallet")?.to_owned();
                        shared
                            .lock()
                            .unwrap()
                            .agents
                            .push(params["agent"].as_str().unwrap_or_default().into());
                        if deny_dev && wallet.starts_with("46fca3") {
                            writeln!(
                                connection.get_mut(),
                                "{}",
                                json!({
                                    "id": request["id"], "error": {"code": -1, "message": "login denied"}
                                })
                            )?;
                            connection.get_mut().flush()?;
                            break;
                        }
                        json!({"id": "rxminer-test-rig", "job": job, "status": "OK",
                            "extensions": ["algo", "nicehash", "keepalive"]})
                    }
                    Some("keepalived") => {
                        shared.lock().unwrap().keepalives += 1;
                        json!({"status": "KEEPALIVED"})
                    }
                    Some("submit") => {
                        if params["algo"] != "rx/0" {
                            error =
                                json!({"code": -1, "message": "missing or incorrect algorithm"});
                            Value::Null
                        } else if params["job_id"] != job["job_id"] {
                            error = json!({"code": -1, "message": "stale job"});
                            Value::Null
                        } else {
                            shared.lock().unwrap().shares.push((
                                wallet.clone(),
                                params["nonce"].as_str().context("share nonce")?.to_owned(),
                            ));
                            json!({"status": "OK"})
                        }
                    }
                    _ => json!({"status": "OK"}),
                };
                let response = json!({"id": request["id"], "jsonrpc": "2.0", "error": error, "result": result});
                writeln!(connection.get_mut(), "{response}")?;
                connection.get_mut().flush()?;
            }
            Ok(())
        })?;
        Ok(Self {
            server,
            state,
            available,
        })
    }
    pub fn port(&self) -> u16 {
        self.server.port
    }
    pub fn snapshot(&self) -> PoolState {
        self.state.lock().unwrap().clone()
    }
    pub fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::Release);
        if !available {
            for socket in self.server.connections.sockets.lock().unwrap().iter() {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    }
}

fn read_header(stream: &mut impl Read) -> Result<String> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        ensure!(bytes.len() < 65536, "HTTP header too large");
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    Ok(String::from_utf8(bytes)?)
}

pub struct Proxy(Server);
impl Proxy {
    pub fn new(socks: bool, pool_port: u16) -> Result<Self> {
        Ok(Self(Server::new(move |mut client, connections| {
            let requested = if socks {
                let mut head = [0; 2];
                client.read_exact(&mut head)?;
                ensure!(head[0] == 5, "SOCKS version");
                let mut methods = vec![0; head[1] as usize];
                client.read_exact(&mut methods)?;
                ensure!(methods.contains(&0), "SOCKS method");
                client.write_all(&[5, 0])?;
                let mut request = [0; 4];
                client.read_exact(&mut request)?;
                ensure!(request[..3] == [5, 1, 0], "SOCKS CONNECT");
                match request[3] {
                    1 => {
                        let mut ip = [0; 4];
                        client.read_exact(&mut ip)?;
                        ensure!(ip == [127, 0, 0, 1], "nonlocal proxy target");
                    }
                    3 => {
                        let mut size = [0];
                        client.read_exact(&mut size)?;
                        let mut host = vec![0; size[0] as usize];
                        client.read_exact(&mut host)?;
                        ensure!(
                            host == b"localhost" || host == b"127.0.0.1",
                            "nonlocal proxy target"
                        );
                    }
                    _ => bail!("unsupported SOCKS address"),
                }
                let mut port = [0; 2];
                client.read_exact(&mut port)?;
                u16::from_be_bytes(port)
            } else {
                let header = read_header(&mut client)?;
                let first: Vec<_> = header
                    .lines()
                    .next()
                    .context("CONNECT line")?
                    .split_whitespace()
                    .collect();
                ensure!(first.len() == 3 && first[0] == "CONNECT", "CONNECT request");
                let (host, port) = first[1].rsplit_once(':').context("CONNECT target")?;
                ensure!(
                    matches!(host, "127.0.0.1" | "localhost"),
                    "nonlocal proxy target"
                );
                port.parse()?
            };
            ensure!(requested == pool_port, "unexpected proxy target port");
            let mut upstream = TcpStream::connect(("127.0.0.1", pool_port))?;
            connections.register(&upstream)?;
            if socks {
                client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])?;
            } else {
                client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
            }
            let mut back_client = client.try_clone()?;
            let mut back_upstream = upstream.try_clone()?;
            thread::scope(|scope| {
                scope.spawn(move || {
                    let _ = std::io::copy(&mut back_upstream, &mut back_client);
                    let _ = back_client.shutdown(Shutdown::Both);
                    let _ = back_upstream.shutdown(Shutdown::Both);
                });
                let _ = std::io::copy(&mut client, &mut upstream);
                let _ = client.shutdown(Shutdown::Both);
                let _ = upstream.shutdown(Shutdown::Both);
            });
            Ok(())
        })?))
    }
    pub fn port(&self) -> u16 {
        self.0.port
    }
}

pub struct Api {
    pub port: u16,
    pub token: String,
    pub certificate: Option<PathBuf>,
}
impl Api {
    pub fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        authenticate: bool,
    ) -> Result<(u16, Value)> {
        let socket = TcpStream::connect_timeout(
            &([127, 0, 0, 1], self.port).into(),
            Duration::from_secs(3),
        )?;
        socket.set_read_timeout(Some(Duration::from_secs(3)))?;
        socket.set_write_timeout(Some(Duration::from_secs(3)))?;
        let mut stream: Box<dyn Stream> = if let Some(cert) = &self.certificate {
            let mut roots = RootCertStore::empty();
            for cert in CertificateDer::pem_file_iter(cert)? {
                roots.add(cert?)?;
            }
            let config = ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth();
            Box::new(StreamOwned::new(
                ClientConnection::new(Arc::new(config), "localhost".try_into()?)?,
                socket,
            ))
        } else {
            Box::new(socket)
        };
        let body = body.map(Value::to_string).unwrap_or_default();
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", body.len())?;
        if authenticate {
            write!(stream, "Authorization: Bearer {}\r\n", self.token)?;
        }
        write!(stream, "\r\n{body}")?;
        stream.flush()?;
        let header = read_header(&mut stream)?;
        let status: u16 = header
            .split_whitespace()
            .nth(1)
            .context("HTTP status")?
            .parse()?;
        let length: usize = header
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim())
            })
            .context("HTTP content length")?
            .parse()?;
        ensure!(length <= 1024 * 1024, "HTTP response too large");
        let mut body = vec![0; length];
        stream.read_exact(&mut body)?;
        Ok((status, serde_json::from_slice(&body)?))
    }
    pub fn get(&self, path: &str) -> Result<Value> {
        let (status, value) = self.request("GET", path, None, true)?;
        ensure!(status == 200, "GET {path}: {status} {value}");
        Ok(value)
    }
    pub fn wait_for(
        &self,
        miner: &mut Process,
        seconds: u64,
        predicate: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        wait_until(seconds, || {
            miner.ensure_running()?;
            Ok(self.get("/summary").ok().filter(&predicate))
        })
    }
}
