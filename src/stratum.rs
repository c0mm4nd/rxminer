use crate::{config::PoolOptions, control::Control, job::Job, proxy};
use anyhow::{bail, ensure, Context, Result};
use arc_swap::ArcSwapOption;
use openssl::{
    hash::MessageDigest,
    ssl::{SslConnector, SslMethod, SslStream, SslVerifyMode, SslVersion},
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const QUEUE_LIMIT: usize = 8192;
const LINE_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub struct ClientState {
    pub connected: AtomicBool,
    nicehash: AtomicBool,
    pub submitted: AtomicU64,
    pub accepted: AtomicU64,
    pub rejected: AtomicU64,
    pub job: ArcSwapOption<Job>,
}
impl ClientState {
    pub fn shares(&self) -> Value {
        json!({"submitted":self.submitted.load(Ordering::Relaxed),
            "accepted":self.accepted.load(Ordering::Relaxed), "rejected":self.rejected.load(Ordering::Relaxed)})
    }
}

struct Share {
    job_id: String,
    session: u64,
    nonce: u32,
    hash: [u8; 32],
}

#[derive(Clone)]
pub struct Submitter {
    sender: SyncSender<Share>,
    state: Arc<ClientState>,
}
impl Submitter {
    pub fn submit(&self, job: &Job, nonce: u32, hash: [u8; 32]) {
        let share = Share {
            job_id: job.id.clone(),
            session: job.session,
            nonce,
            hash,
        };
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
            self.sender.try_send(share)
        {
            self.state.rejected.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub struct Client {
    pub state: Arc<ClientState>,
    pub submitter: Submitter,
    stop: Arc<ConnectionStop>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct ConnectionStop {
    stopped: AtomicBool,
    socket: Mutex<Option<TcpStream>>,
}
impl ConnectionStop {
    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    fn register(&self, stream: &TcpStream) -> Result<()> {
        let mut socket = self.socket.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(!self.stopped(), "connection interrupted");
        *socket = Some(stream.try_clone()?);
        Ok(())
    }
    fn clear(&self) {
        self.socket.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        let socket = self.socket.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(socket) = socket.as_ref() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}
impl Client {
    pub fn start(options: PoolOptions, dev: bool, control: Control) -> Result<Self> {
        let state = Arc::new(ClientState::default());
        let (sender, receiver) = mpsc::sync_channel(QUEUE_LIMIT);
        let submitter = Submitter {
            sender,
            state: state.clone(),
        };
        let stop = Arc::new(ConnectionStop::default());
        let thread_state = state.clone();
        let thread_stop = stop.clone();
        let thread = thread::Builder::new()
            .name(if dev { "stratum-dev" } else { "stratum-user" }.into())
            .spawn(move || {
                let _panic = control.watch_thread("Stratum worker");
                let mut session = 0;
                let mut failures = 0;
                while !thread_stop.stopped() && !control.stopped() {
                    session += 1;
                    let result = session_loop(
                        &options,
                        dev,
                        session,
                        &thread_state,
                        &receiver,
                        &thread_stop,
                        &control,
                    );
                    if thread_state.connected.load(Ordering::Acquire) {
                        failures = 0;
                    }
                    failures += 1;
                    thread_stop.clear();
                    thread_state.connected.store(false, Ordering::Release);
                    thread_state.job.store(None);
                    if let Err(error) = result {
                        if !control.stopped() && !thread_stop.stopped() {
                            crate::status!(
                                "[net] {} connection: {error:#}",
                                if dev { "developer" } else { "user" }
                            );
                            if dev {
                                control
                                    .fail(format!("developer connection unavailable: {error:#}"));
                                break;
                            }
                        }
                    }
                    if !dev
                        && failures >= options.retries
                        && !control.stopped()
                        && !thread_stop.stopped()
                    {
                        control.fail("user pool retries exhausted");
                        break;
                    }
                    for _ in 0..options.retry_pause.saturating_mul(10) {
                        if control.stopped() || thread_stop.stopped() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            })
            .context("spawn Stratum thread")?;
        Ok(Self {
            state,
            submitter,
            stop,
            thread: Some(thread),
        })
    }
    pub fn stop(&mut self) {
        self.stop.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop();
    }
}

enum Transport {
    Tcp(TcpStream),
    Tls(SslStream<TcpStream>),
}
impl Read for Transport {
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(data),
            Self::Tls(s) => s.read(data),
        }
    }
}
impl Write for Transport {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(s) => s.write(data),
            Self::Tls(s) => s.write(data),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.flush(),
            Self::Tls(s) => s.flush(),
        }
    }
}
impl Transport {
    fn connect(options: &PoolOptions, stop: &ConnectionStop, control: &Control) -> Result<Self> {
        let tcp = proxy::connect_interruptible(
            &options.address,
            options.proxy.as_ref(),
            options.dns,
            |stream| stop.register(stream),
            || stop.stopped() || control.stopped(),
        )?;
        if !options.address.tls {
            tcp.set_read_timeout(Some(Duration::from_millis(100)))?;
            crate::status!("[net] TCP connected to {}", options.address.authority());
            return Ok(Self::Tcp(tcp));
        }
        let mut connector = SslConnector::builder(SslMethod::tls_client())?;
        connector.set_min_proto_version(Some(SslVersion::TLS1_2))?;
        connector.set_verify(if options.tls.verify {
            SslVerifyMode::PEER
        } else {
            SslVerifyMode::NONE
        });
        if !options.tls.ca.is_empty() {
            connector
                .set_ca_file(&options.tls.ca)
                .context("load TLS CA")?;
        }
        let mut ssl = connector.build().configure()?;
        ssl.set_verify_hostname(options.tls.verify);
        let stream = ssl
            .connect(&options.address.host, tcp)
            .context("TLS handshake")?;
        if !options.tls.fingerprint.is_empty() {
            let cert = stream
                .ssl()
                .peer_certificate()
                .context("pool did not present a certificate")?;
            let digest = cert.digest(MessageDigest::sha256())?;
            ensure!(
                hex::encode(digest) == options.tls.fingerprint,
                "TLS certificate fingerprint mismatch"
            );
        }
        stream
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(100)))?;
        crate::status!(
            "[net] TLS connected to {} ({})",
            options.address.authority(),
            stream.ssl().version_str()
        );
        Ok(Self::Tls(stream))
    }

    fn send(&mut self, value: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        self.write_all(&bytes).context("write Stratum request")
    }
}

fn session_loop(
    options: &PoolOptions,
    dev: bool,
    session: u64,
    state: &ClientState,
    receiver: &Receiver<Share>,
    stop: &ConnectionStop,
    control: &Control,
) -> Result<()> {
    state.nicehash.store(options.nicehash, Ordering::Relaxed);
    let mut transport = Transport::connect(options, stop, control)?;
    transport.send(json!({"id":1,"jsonrpc":"2.0","method":"login","params":{
        "login":options.user, "pass":options.pass, "rigid":options.rig_id,
        "agent":options.user_agent, "algo":["rx/0"]}}))?;
    let mut login_id = None;
    let mut pending = HashSet::new();
    let mut next_id = 2u64;
    let mut buffer = Vec::new();
    let mut last_keepalive = Instant::now();
    let login_deadline = Instant::now() + Duration::from_secs(10);
    let result = (|| -> Result<()> {
        while !stop.stopped() && !control.stopped() {
            ensure!(
                login_id.is_some() || Instant::now() < login_deadline,
                "pool login timed out"
            );
            if let Some(login_id) = &login_id {
                for _ in 0..64 {
                    if pending.len() >= QUEUE_LIMIT {
                        break;
                    }
                    let Ok(share) = receiver.try_recv() else {
                        break;
                    };
                    if share.session != session {
                        state.rejected.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let id = next_id;
                    next_id += 1;
                    transport.send(json!({"id":id,"jsonrpc":"2.0","method":"submit","params":{
                        "id":login_id,"job_id":share.job_id,"nonce":hex::encode(share.nonce.to_le_bytes()),"result":hex::encode(share.hash)}}))?;
                    pending.insert(id);
                    state.submitted.fetch_add(1, Ordering::Relaxed);
                }
            }
            if login_id.is_some()
                && options.keepalive > 0
                && last_keepalive.elapsed() >= Duration::from_secs(options.keepalive)
            {
                transport.send(json!({"id":next_id,"jsonrpc":"2.0","method":"keepalived","params":{"id":login_id}}))?;
                next_id += 1;
                last_keepalive = Instant::now();
            }
            let mut chunk = [0; 32768];
            let size = match transport.read(&mut chunk) {
                Ok(0) => bail!("pool connection closed"),
                Ok(size) => size,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue
                }
                Err(error) => return Err(error).context("read Stratum response"),
            };
            buffer.extend_from_slice(&chunk[..size]);
            ensure!(buffer.len() <= LINE_LIMIT, "pool sent an oversized line");
            let mut consumed = 0;
            while let Some(newline) = buffer[consumed..].iter().position(|byte| *byte == b'\n') {
                let end = consumed + newline;
                let line = &buffer[consumed..end];
                if !line.iter().all(u8::is_ascii_whitespace) {
                    let message: Value =
                        serde_json::from_slice(line).context("invalid pool JSON")?;
                    handle_message(
                        message,
                        (dev, options.nicehash),
                        session,
                        state,
                        &mut login_id,
                        &mut pending,
                    )?;
                }
                consumed = end + 1;
            }
            buffer.drain(..consumed);
        }
        Ok(())
    })();
    state
        .rejected
        .fetch_add(pending.len() as u64, Ordering::Relaxed);
    result
}

fn handle_message(
    message: Value,
    mode: (bool, bool),
    session: u64,
    state: &ClientState,
    login_id: &mut Option<String>,
    pending: &mut HashSet<u64>,
) -> Result<()> {
    let (dev, nicehash) = mode;
    if let Some(method) = message.get("method").and_then(Value::as_str) {
        if method == "job" {
            ensure!(login_id.is_some(), "job notification before login");
            let mut job = Job::parse(message["params"].clone(), dev, session)?;
            job.nicehash = nicehash || state.nicehash.load(Ordering::Relaxed);
            crate::status!("[net] new job {} height {}", job.id, job.height);
            state.job.store(Some(Arc::new(job)));
        }
        return Ok(());
    }
    let id = message
        .get("id")
        .and_then(Value::as_u64)
        .context("invalid response id")?;
    let failed = message.get("error").is_some_and(|error| !error.is_null());
    if id == 1 {
        ensure!(!failed, "pool rejected login: {}", message["error"]);
        let result = &message["result"];
        let id = result["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .context("login response has no session id")?;
        let mut job = Job::parse(result["job"].clone(), dev, session)?;
        job.nicehash = nicehash
            || result["extensions"]
                .as_array()
                .is_some_and(|xs| xs.iter().any(|x| x == "nicehash"));
        state.nicehash.store(job.nicehash, Ordering::Relaxed);
        *login_id = Some(id.into());
        state.job.store(Some(Arc::new(job)));
        state.connected.store(true, Ordering::Release);
        crate::status!(
            "[net] {} login accepted",
            if dev { "developer" } else { "user" }
        );
    } else if pending.remove(&id) {
        if failed {
            state.rejected.fetch_add(1, Ordering::Relaxed);
        } else {
            state.accepted.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Cli, Options};
    use clap::Parser;
    use std::{net::TcpListener, sync::Barrier};

    #[test]
    fn stop_racing_socket_registration_never_leaves_a_live_socket() {
        for _ in 0..64 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (mut peer, _) = listener.accept().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            let stop = ConnectionStop::default();
            let barrier = Barrier::new(2);
            thread::scope(|scope| {
                scope.spawn(|| {
                    barrier.wait();
                    let _ = stop.register(&stream);
                });
                barrier.wait();
                stop.stop();
            });
            // If stop won the race, registration must reject and the caller's
            // stream is dropped; otherwise shutdown wakes all socket clones.
            drop(stream);
            assert_eq!(peer.read(&mut [0; 1]).unwrap(), 0);
        }
    }

    #[test]
    fn stop_interrupts_stalled_tls_and_proxy_handshakes() {
        for proxy in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let pool = format!("stratum+ssl://{address}");
            let proxy_url = format!("http://{address}");
            let mut args = vec!["rxminer", "-o", &pool, "-u", "test", "--no-proxy"];
            if proxy {
                args.extend(["--proxy", &proxy_url]);
            }
            let options = Options::from_cli(Cli::try_parse_from(args).unwrap()).unwrap();
            let control = Control::new(Arc::new(AtomicBool::new(false)));
            let mut client = Client::start(options.pool.unwrap(), false, control).unwrap();
            let (mut peer, _) = listener.accept().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            assert!(peer.read(&mut [0; 1024]).unwrap() > 0); // handshake began
            let start = Instant::now();
            client.stop();
            assert!(start.elapsed() < Duration::from_secs(2), "shutdown stalled");
            assert!(!client.state.connected.load(Ordering::Acquire));
        }
    }

    #[test]
    fn rejected_login_and_submit_responses() {
        let state = ClientState::default();
        let mut login = None;
        let mut pending = HashSet::from([2, 3]);
        assert!(handle_message(
            json!({"id":1,"error":{"message":"denied"}}),
            (true, false),
            1,
            &state,
            &mut login,
            &mut pending
        )
        .is_err());
        handle_message(
            json!({"id":2,"error":{"message":"stale"}}),
            (false, false),
            1,
            &state,
            &mut login,
            &mut pending,
        )
        .unwrap();
        handle_message(
            json!({"id":3,"error":null,"result":{"status":"OK"}}),
            (false, false),
            1,
            &state,
            &mut login,
            &mut pending,
        )
        .unwrap();
        assert_eq!(state.rejected.load(Ordering::Relaxed), 1);
        assert_eq!(state.accepted.load(Ordering::Relaxed), 1);
        assert!(pending.is_empty());
    }
}
