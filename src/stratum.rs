use crate::{config::PoolOptions, control::Control, job::Job, proxy};
use anyhow::{bail, ensure, Context, Result};
use arc_swap::ArcSwapOption;
use openssl::{
    hash::MessageDigest,
    ssl::{HandshakeError, SslConnector, SslMethod, SslStream, SslVerifyMode, SslVersion},
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
    pub endpoint: Mutex<String>,
    nicehash: AtomicBool,
    algo: AtomicBool,
    keepalive: AtomicBool,
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
        Self::start_pools(vec![options], dev, control)
    }

    pub(crate) fn start_developer(pools: Vec<PoolOptions>, control: Control) -> Result<Self> {
        Self::start_pools(pools, true, control)
    }

    fn start_pools(pools: Vec<PoolOptions>, dev: bool, control: Control) -> Result<Self> {
        ensure!(!pools.is_empty(), "no pool endpoints");
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
                let mut pool_index = 0;
                while !thread_stop.stopped() && !control.stopped() {
                    let options = &pools[pool_index];
                    *thread_state
                        .endpoint
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = options.address.url.clone();
                    session += 1;
                    let result = session_loop(
                        options,
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
                            if dev {
                                if failures >= pools.len() {
                                    control.fail("required pool connection unavailable");
                                    break;
                                }
                                pool_index = (pool_index + 1) % pools.len();
                                crate::status!(
                                    "[net] developer connection: {error:#}; switching to {}",
                                    pools[pool_index].address.authority()
                                );
                            } else {
                                crate::status!("[net] user connection: {error:#}");
                            }
                        }
                    }
                    if !dev
                        && failures >= options.retries as usize
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
    fn connect(
        options: &PoolOptions,
        stop: &ConnectionStop,
        control: &Control,
        dev: bool,
    ) -> Result<Self> {
        let tcp = proxy::connect_interruptible(
            &options.address,
            options.proxy.as_ref(),
            options.dns,
            |stream| stop.register(stream),
            || stop.stopped() || control.stopped(),
        )?;
        if !options.address.tls {
            tcp.set_read_timeout(Some(Duration::from_millis(100)))?;
            if !dev {
                crate::status!("[net] TCP connected to {}", options.address.authority());
            }
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
        // Resume the same TLS state after WouldBlock. Polling avoids relying
        // on cross-thread shutdown to wake a blocking read on every OS.
        tcp.set_nonblocking(true)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut handshake = ssl.connect(&options.address.host, tcp);
        let stream = loop {
            ensure!(
                !stop.stopped() && !control.stopped(),
                "TLS handshake interrupted"
            );
            match handshake {
                Ok(stream) => break stream,
                Err(HandshakeError::WouldBlock(pending)) => {
                    ensure!(Instant::now() < deadline, "TLS handshake timed out");
                    thread::sleep(Duration::from_millis(10));
                    handshake = pending.handshake();
                }
                Err(error) => return Err(error).context("TLS handshake"),
            }
        };
        stream.get_ref().set_nonblocking(false)?;
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
        if !dev {
            crate::status!(
                "[net] TLS connected to {} ({})",
                options.address.authority(),
                stream.ssl().version_str()
            );
        }
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
    state.algo.store(false, Ordering::Relaxed);
    state.keepalive.store(false, Ordering::Relaxed);
    let mut transport = Transport::connect(options, stop, control, dev)?;
    transport.send(json!({"id":1,"jsonrpc":"2.0","method":"login","params":{
        "login":options.user, "pass":options.pass, "rigid":options.rig_id,
        "agent":options.user_agent, "algo":["rx/0"]}}))?;
    let mut login_id: Option<String> = None;
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
                    transport.send(share_request(id, login_id, &share, state))?;
                    pending.insert(id);
                    state.submitted.fetch_add(1, Ordering::Relaxed);
                }
            }
            let keepalive = keepalive_interval(options, state);
            if login_id.is_some()
                && keepalive > 0
                && last_keepalive.elapsed() >= Duration::from_secs(keepalive)
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

fn keepalive_interval(options: &PoolOptions, state: &ClientState) -> u64 {
    if options.keepalive_auto && state.keepalive.load(Ordering::Relaxed) {
        60
    } else {
        options.keepalive
    }
}

fn share_request(id: u64, login_id: &str, share: &Share, state: &ClientState) -> Value {
    let mut request = json!({"id":id,"jsonrpc":"2.0","method":"submit","params":{
        "id":login_id,"job_id":share.job_id,"nonce":hex::encode(share.nonce.to_le_bytes()),"result":hex::encode(share.hash)}});
    if state.algo.load(Ordering::Relaxed) {
        request["params"]["algo"] = "rx/0".into();
    }
    request
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
            if !dev {
                crate::status!("[net] new job {} height {}", job.id, job.height);
            }
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
        let extension = |name: &str| {
            result["extensions"]
                .as_array()
                .is_some_and(|xs| xs.iter().any(|x| x == name))
        };
        job.nicehash = nicehash || extension("nicehash");
        state.nicehash.store(job.nicehash, Ordering::Relaxed);
        state.algo.store(extension("algo"), Ordering::Relaxed);
        state
            .keepalive
            .store(extension("keepalive"), Ordering::Relaxed);
        *login_id = Some(id.into());
        state.job.store(Some(Arc::new(job)));
        state.connected.store(true, Ordering::Release);
        if !dev {
            crate::status!("[net] user login accepted");
        }
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

    fn await_condition(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < deadline, "Stratum test timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn developer_failover_keeps_session_boundaries_and_reports_endpoint() {
        use std::io::{BufRead, BufReader};
        let mut pools = Vec::new();
        let mut servers = Vec::new();
        let mut releases = Vec::new();
        for number in 1..=2u32 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap().to_string();
            let mut pool = Options::from_cli(
                Cli::try_parse_from([
                    "rxminer",
                    "-o",
                    &address,
                    "-u",
                    crate::miner::DEV_ADDRESS,
                    "--no-proxy",
                ])
                .unwrap(),
            )
            .unwrap()
            .pool
            .unwrap();
            pool.retry_pause = 0;
            pools.push(pool);
            let (release, receiver) = mpsc::channel();
            releases.push(release);
            servers.push(thread::spawn(move || {
                let mut socket = None;
                await_condition(|| match listener.accept() {
                    Ok((peer, _)) => {
                        socket = Some(peer);
                        true
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => false,
                    Err(e) => panic!("accept: {e}"),
                });
                let socket = socket.unwrap();
                // Accepted sockets can inherit the listener's nonblocking mode
                // on macOS/Windows. This fixture uses blocking, bounded reads.
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut peer = BufReader::new(socket);
                let mut line = String::new();
                peer.read_line(&mut line).unwrap();
                let login: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(login["params"]["login"], crate::miner::DEV_ADDRESS);
                let response = json!({"id":1,"result":{"id":format!("session-{number}"),
                    "extensions":["algo"], "job":{"job_id":"same-id", "blob":"00".repeat(76),
                    "seed_hash":"00".repeat(32), "target":"ffffffff"}}});
                writeln!(peer.get_mut(), "{response}").unwrap();
                line.clear();
                peer.read_line(&mut line).unwrap();
                let share: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(share["method"], "submit");
                assert_eq!(share["params"]["id"], format!("session-{number}"));
                assert_eq!(share["params"]["nonce"], hex::encode(number.to_le_bytes()));
                assert_eq!(share["params"]["algo"], "rx/0");
                writeln!(
                    peer.get_mut(),
                    "{}",
                    json!({"id":share["id"],"error":null,"result":{"status":"OK"}})
                )
                .unwrap();
                receiver.recv_timeout(Duration::from_secs(5)).unwrap();
                peer.get_mut().shutdown(Shutdown::Both).unwrap();
            }));
        }
        let backup = pools[1].address.url.clone();
        let control = Control::new(Arc::new(AtomicBool::new(false)));
        let mut client = Client::start_developer(pools, control.clone()).unwrap();
        await_condition(|| client.state.connected.load(Ordering::Acquire));
        let old = client.state.job.load_full().unwrap();
        client.submitter.submit(&old, 1, [0; 32]);
        await_condition(|| client.state.accepted.load(Ordering::Relaxed) == 1);
        releases[0].send(()).unwrap();
        await_condition(|| {
            client
                .state
                .job
                .load_full()
                .is_some_and(|job| job.session != old.session)
        });
        let current = client.state.job.load_full().unwrap();
        client.submitter.submit(&old, 99, [0; 32]);
        client.submitter.submit(&current, 2, [0; 32]);
        await_condition(|| client.state.accepted.load(Ordering::Relaxed) == 2);
        assert_eq!(client.state.rejected.load(Ordering::Relaxed), 1);
        assert_eq!(*client.state.endpoint.lock().unwrap(), backup);
        assert!(!control.stopped());
        client.stop();
        releases[1].send(()).unwrap();
        for server in servers {
            server.join().unwrap();
        }
    }

    #[test]
    fn exhausted_developer_endpoints_stop_mining() {
        let listeners: Vec<_> = (0..2)
            .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
            .collect();
        let pools = listeners
            .iter()
            .map(|listener| {
                let address = listener.local_addr().unwrap().to_string();
                let mut pool = Options::from_cli(
                    Cli::try_parse_from(["rxminer", "-o", &address, "-u", "test", "--no-proxy"])
                        .unwrap(),
                )
                .unwrap()
                .pool
                .unwrap();
                pool.retry_pause = 0;
                pool
            })
            .collect();
        drop(listeners);
        let control = Control::new(Arc::new(AtomicBool::new(false)));
        let mut client = Client::start_developer(pools, control.clone()).unwrap();
        await_condition(|| control.stopped());
        assert_eq!(
            control.error().as_deref(),
            Some("required pool connection unavailable")
        );
        assert!(!client.state.connected.load(Ordering::Acquire));
        assert!(client.state.job.load_full().is_none());
        client.stop();
    }

    #[test]
    fn negotiated_extensions_control_submits_and_idle_keepalives() {
        let state = ClientState::default();
        let mut login = None;
        let mut pending = HashSet::new();
        let options = Options::from_cli(
            Cli::try_parse_from([
                "rxminer",
                "-o",
                "localhost:3333",
                "-u",
                "test",
                "--no-proxy",
            ])
            .unwrap(),
        )
        .unwrap();
        let mut pool = options.pool.unwrap();
        let share = Share {
            job_id: "job".into(),
            session: 1,
            nonce: 0xab123456,
            hash: [0x12; 32],
        };
        // A reconnect to a server without extensions must clear the old capabilities.
        for extensions in [json!(["algo", "nicehash", "keepalive"]), json!([])] {
            let advertised = !extensions.as_array().unwrap().is_empty();
            handle_message(json!({"id":1,"result":{"id":"session", "extensions":extensions,
                "job":{"job_id":"job", "blob":"00".repeat(76), "seed_hash":"00".repeat(32), "target":"ffffffff"}}}),
                (false, false), 1, &state, &mut login, &mut pending).unwrap();
            let request = share_request(2, "session", &share, &state);
            assert_eq!(request["params"]["nonce"], "563412ab");
            assert_eq!(request["params"].get("algo").is_some(), advertised);
            if advertised {
                assert_eq!(request["params"]["algo"], "rx/0");
            }
            assert_eq!(state.job.load_full().unwrap().nicehash, advertised);
            pool.keepalive_auto = true;
            pool.keepalive = 0;
            assert_eq!(
                keepalive_interval(&pool, &state),
                if advertised { 60 } else { 0 }
            );
            pool.keepalive_auto = false;
            assert_eq!(keepalive_interval(&pool, &state), 0);
            pool.keepalive = 7;
            assert_eq!(keepalive_interval(&pool, &state), 7);
        }
    }

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
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "shutdown stalled (proxy={proxy}): {:?}",
                start.elapsed()
            );
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
