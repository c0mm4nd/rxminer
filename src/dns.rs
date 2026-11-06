use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{mpsc, OnceLock},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub ip_version: u8,
    pub ttl: u64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            ip_version: 0,
            ttl: 30,
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.ip_version, 0 | 4 | 6),
            "dns.ip_version must be 0, 4 or 6"
        );
        ensure!(self.ttl <= 86400, "dns.ttl must be 0..86400 seconds");
        Ok(())
    }
    fn accepts(&self, address: &SocketAddr) -> bool {
        self.ip_version == 0 || (self.ip_version == 4) == address.is_ipv4()
    }
}
type CacheKey = (String, u16, u8, u64);
type CacheEntry = (Instant, Vec<SocketAddr>);

struct Request {
    host: String,
    port: u16,
    options: Options,
    response: mpsc::SyncSender<std::io::Result<Vec<SocketAddr>>>,
    deadline: Instant,
}
// One bounded worker prevents a stuck system resolver from spawning unbounded
// threads. Callers poll their cancellation flag; SOCKS5h bypasses target lookup.
fn worker() -> &'static mpsc::SyncSender<Request> {
    static WORKER: OnceLock<mpsc::SyncSender<Request>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel::<Request>(32);
        thread::Builder::new()
            .name("dns-resolver".into())
            .spawn(move || {
                let mut cache: HashMap<CacheKey, CacheEntry> = HashMap::new();
                while let Ok(request) = receiver.recv() {
                    let now = Instant::now();
                    if now >= request.deadline {
                        continue;
                    }
                    cache.retain(|_, (expires, _)| *expires > now);
                    let key = (
                        request.host.clone(),
                        request.port,
                        request.options.ip_version,
                        request.options.ttl,
                    );
                    if request.options.ttl > 0 {
                        if let Some((_, addresses)) = cache.get(&key) {
                            let _ = request.response.send(Ok(addresses.clone()));
                            continue;
                        }
                    }
                    let result =
                        (request.host.as_str(), request.port)
                            .to_socket_addrs()
                            .map(|all| {
                                all.filter(|a| request.options.accepts(a))
                                    .collect::<Vec<_>>()
                            });
                    if let Ok(addresses) = &result {
                        if request.options.ttl > 0 && !addresses.is_empty() && cache.len() < 128 {
                            cache.insert(
                                key,
                                (
                                    Instant::now() + Duration::from_secs(request.options.ttl),
                                    addresses.clone(),
                                ),
                            );
                        }
                    }
                    let _ = request.response.send(result);
                }
            })
            .expect("spawn bounded DNS resolver");
        sender
    })
}

pub fn resolve(
    host: &str,
    port: u16,
    options: Options,
    stopped: impl Fn() -> bool,
) -> Result<Vec<SocketAddr>> {
    options.validate()?;
    ensure!(!stopped(), "DNS lookup interrupted");
    if let Ok(ip) = host.parse::<IpAddr>() {
        let address = SocketAddr::new(ip, port);
        ensure!(
            options.accepts(&address),
            "address conflicts with dns.ip_version"
        );
        return Ok(vec![address]);
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let deadline = Instant::now() + Duration::from_secs(5);
    worker()
        .try_send(Request {
            host: host.into(),
            port,
            options,
            response: sender,
            deadline,
        })
        .map_err(|_| anyhow::anyhow!("DNS resolver busy"))?;
    loop {
        ensure!(!stopped(), "DNS lookup interrupted");
        ensure!(Instant::now() < deadline, "DNS lookup timed out");
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(result) => {
                let addresses = result.context("DNS lookup failed")?;
                ensure!(
                    !addresses.is_empty(),
                    "DNS lookup returned no matching addresses"
                );
                return Ok(addresses);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => bail!("DNS resolver stopped"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn address_family_and_cancellation_are_enforced() {
        for (host, family) in [("127.0.0.1", 4), ("::1", 6)] {
            assert_eq!(
                resolve(
                    host,
                    123,
                    Options {
                        ip_version: family,
                        ttl: 0
                    },
                    || false
                )
                .unwrap()[0]
                    .port(),
                123
            );
            assert!(resolve(
                host,
                123,
                Options {
                    ip_version: 10 - family,
                    ttl: 30
                },
                || false
            )
            .is_err());
        }
        assert!(resolve("never-resolve.invalid", 1, Options::default(), || true).is_err());
        for ttl in [0, 30] {
            let values =
                resolve("localhost", 123, Options { ip_version: 4, ttl }, || false).unwrap();
            assert!(values.iter().all(SocketAddr::is_ipv4));
        }
    }
}
