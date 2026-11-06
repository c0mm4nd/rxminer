use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use percent_encoding::percent_decode_str;
use std::{
    env,
    io::{self, Read, Write},
    net::{IpAddr, TcpStream},
    thread,
    time::{Duration, Instant},
};
use url::Url;

#[derive(Clone, Debug)]
pub struct PoolAddress {
    pub url: String,
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

fn host(url: &Url) -> Result<String> {
    Ok(url
        .host_str()
        .context("URL has no host")?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .into())
}

impl PoolAddress {
    pub fn parse(text: &str) -> Result<Self> {
        let text = if text.contains("://") {
            text.to_owned()
        } else {
            format!("stratum+tcp://{text}")
        };
        let url = Url::parse(&text).context("invalid pool URL")?;
        ensure!(
            matches!(
                url.scheme(),
                "stratum+ssl" | "stratum+tls" | "stratum+tcp" | "stratum"
            ),
            "unsupported pool scheme"
        );
        ensure!(
            url.path().is_empty()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none(),
            "pool URL must contain only a host and port"
        );
        let port = url.port().unwrap_or(3333);
        ensure!(port != 0, "pool port must be nonzero");
        Ok(Self {
            url: text,
            host: host(&url)?,
            port,
            tls: matches!(url.scheme(), "stratum+ssl" | "stratum+tls"),
        })
    }

    pub fn authority(&self) -> String {
        authority(&self.host, self.port)
    }
}

fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Http,
    Socks5,
    Socks5h,
}

#[derive(Clone)]
pub struct Proxy {
    kind: Kind,
    host: String,
    port: u16,
    user: String,
    pass: String,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

impl Proxy {
    pub fn parse(text: &str) -> Result<Self> {
        let text = if text.contains("://") {
            text.to_string()
        } else {
            format!("http://{text}")
        };
        let url = Url::parse(&text).context("invalid proxy URL")?;
        let kind = match url.scheme() {
            "http" => Kind::Http,
            "socks5" => Kind::Socks5,
            "socks5h" => Kind::Socks5h,
            "https" => bail!("TLS-to-proxy is not supported; use http:// CONNECT"),
            _ => bail!("unsupported proxy scheme"),
        };
        ensure!(
            matches!(url.path(), "" | "/") && url.query().is_none() && url.fragment().is_none(),
            "proxy URL must not contain a path"
        );
        // Url normalizes an explicit HTTP :80 to None; preserve that choice
        // while retaining the miner's legacy 8080 default for an omitted port.
        let explicit_port = text
            .split_once("://")
            .and_then(|(_, rest)| rest.split('/').next())
            .and_then(|authority| authority.rsplit('@').next())
            .and_then(|authority| authority.rsplit_once(':'))
            .and_then(|(_, port)| port.parse::<u16>().ok());
        let port = explicit_port
            .or(url.port())
            .unwrap_or(if kind == Kind::Http { 8080 } else { 1080 });
        ensure!(port != 0, "proxy port must be nonzero");
        let user = percent_decode_str(url.username())
            .decode_utf8()?
            .into_owned();
        let pass = percent_decode_str(url.password().unwrap_or(""))
            .decode_utf8()?
            .into_owned();
        ensure!(
            user.len() <= 255 && pass.len() <= 255,
            "proxy credentials too long"
        );
        Ok(Self {
            kind,
            host: host(&url)?,
            port,
            user,
            pass,
        })
    }

    pub fn from_environment(target: &PoolAddress) -> Result<Option<Self>> {
        let bypass = env::var("NO_PROXY")
            .or_else(|_| env::var("no_proxy"))
            .unwrap_or_default();
        if no_proxy_matches(&bypass, &target.host, target.port) {
            return Ok(None);
        }
        for key in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
            if let Ok(value) = env::var(key) {
                if !value.is_empty() {
                    return Self::parse(&value)
                        .map(Some)
                        .with_context(|| format!("invalid {key}"));
                }
            }
        }
        Ok(None)
    }

    pub fn label(&self) -> String {
        let scheme = match self.kind {
            Kind::Http => "http",
            Kind::Socks5 => "socks5",
            Kind::Socks5h => "socks5h",
        };
        format!("{scheme}://{}", authority(&self.host, self.port))
    }

    fn handshake(
        &self,
        stream: &mut (impl Read + Write),
        target: &PoolAddress,
        dns: crate::dns::Options,
        stopped: impl Fn() -> bool,
    ) -> Result<()> {
        if self.kind == Kind::Http {
            let authority = target.authority();
            let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: Keep-Alive\r\n");
            if !self.user.is_empty() {
                request.push_str(&format!(
                    "Proxy-Authorization: Basic {}\r\n",
                    STANDARD.encode(format!("{}:{}", self.user, self.pass))
                ));
            }
            request.push_str("\r\n");
            stream.write_all(request.as_bytes())?;
            // Read exactly through the header, leaving tunneled bytes untouched.
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                ensure!(header.len() < 16384, "proxy response headers too large");
                let mut byte = [0];
                stream.read_exact(&mut byte)?;
                header.push(byte[0]);
            }
            let response = String::from_utf8(header).context("invalid proxy response")?;
            let mut status = response
                .lines()
                .next()
                .unwrap_or_default()
                .split_whitespace();
            ensure!(
                status.next().is_some_and(|s| s.starts_with("HTTP/"))
                    && status.next() == Some("200"),
                "proxy CONNECT rejected"
            );
            return Ok(());
        }
        stream.write_all(if self.user.is_empty() {
            &[5, 1, 0]
        } else {
            &[5, 2, 0, 2]
        })?;
        let mut reply = [0; 2];
        stream.read_exact(&mut reply)?;
        ensure!(reply[0] == 5, "invalid SOCKS version");
        match reply[1] {
            0 => {}
            2 if !self.user.is_empty() => {
                let mut auth = vec![1, self.user.len() as u8];
                auth.extend(self.user.as_bytes());
                auth.push(self.pass.len() as u8);
                auth.extend(self.pass.as_bytes());
                stream.write_all(&auth)?;
                stream.read_exact(&mut reply)?;
                ensure!(reply == [1, 0], "SOCKS5 authentication failed");
            }
            _ => bail!("SOCKS5 selected an unsupported authentication method"),
        }
        let ip = target.host.parse::<IpAddr>().ok().or_else(|| {
            if self.kind == Kind::Socks5 {
                crate::dns::resolve(&target.host, target.port, dns, &stopped)
                    .ok()?
                    .into_iter()
                    .next()
                    .map(|a| a.ip())
            } else {
                None
            }
        });
        ensure!(
            self.kind != Kind::Socks5 || ip.is_some(),
            "local SOCKS5 DNS lookup failed"
        );
        let mut request = vec![5, 1, 0];
        match ip {
            Some(IpAddr::V4(ip)) => {
                request.push(1);
                request.extend(ip.octets());
            }
            Some(IpAddr::V6(ip)) => {
                request.push(4);
                request.extend(ip.octets());
            }
            None => {
                ensure!(target.host.len() <= 255, "SOCKS hostname too long");
                request.extend([3, target.host.len() as u8]);
                request.extend(target.host.as_bytes());
            }
        }
        request.extend(target.port.to_be_bytes());
        stream.write_all(&request)?;
        let mut reply = [0; 4];
        stream.read_exact(&mut reply)?;
        ensure!(
            reply[0] == 5 && reply[1] == 0 && reply[2] == 0,
            "SOCKS5 CONNECT failed (code {})",
            reply[1]
        );
        let len = match reply[3] {
            1 => 4,
            4 => 16,
            3 => {
                let mut len = [0];
                stream.read_exact(&mut len)?;
                len[0] as usize
            }
            _ => bail!("invalid SOCKS address type"),
        };
        stream.read_exact(&mut vec![0; len + 2])?;
        Ok(())
    }
}

pub fn connect(target: &PoolAddress, proxy: Option<&Proxy>) -> Result<TcpStream> {
    connect_interruptible(
        target,
        proxy,
        crate::dns::Options::default(),
        |_| Ok(()),
        || false,
    )
}

// Handshake reads preserve partial replies while polling cancellation. In
// particular, Windows shutdown on a cloned socket need not wake a blocked read.
struct HandshakeIo<'a> {
    stream: &'a TcpStream,
    stopped: &'a dyn Fn() -> bool,
    deadline: Instant,
}
impl HandshakeIo<'_> {
    fn perform<T>(&self, mut operation: impl FnMut(&TcpStream) -> io::Result<T>) -> io::Result<T> {
        loop {
            if (self.stopped)() {
                return Err(io::Error::other("proxy handshake interrupted"));
            }
            if Instant::now() >= self.deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "proxy handshake timed out",
                ));
            }
            match operation(self.stream) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                result => return result,
            }
        }
    }
}
impl Read for HandshakeIo<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.perform(|mut stream| stream.read(buffer))
    }
}
impl Write for HandshakeIo<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.perform(|mut stream| stream.write(buffer))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(()) // TcpStream has no user-space write buffer.
    }
}

pub(crate) fn connect_interruptible(
    target: &PoolAddress,
    proxy: Option<&Proxy>,
    dns: crate::dns::Options,
    register: impl Fn(&TcpStream) -> Result<()>,
    stopped: impl Fn() -> bool,
) -> Result<TcpStream> {
    ensure!(!stopped(), "connection interrupted");
    let (host, port) = proxy
        .map(|p| (p.host.as_str(), p.port))
        .unwrap_or((&target.host, target.port));
    let mut last = None;
    for address in crate::dns::resolve(host, port, dns, &stopped)? {
        ensure!(!stopped(), "connection interrupted");
        match TcpStream::connect_timeout(&address, Duration::from_secs(5)) {
            Ok(stream) => {
                ensure!(!stopped(), "connection interrupted");
                register(&stream)?;
                stream.set_nodelay(true)?;
                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                if let Some(proxy) = proxy {
                    stream.set_nonblocking(true)?;
                    proxy.handshake(
                        &mut HandshakeIo {
                            stream: &stream,
                            stopped: &stopped,
                            deadline: Instant::now() + Duration::from_secs(5),
                        },
                        target,
                        dns,
                        &stopped,
                    )?;
                    stream.set_nonblocking(false)?;
                }
                return Ok(stream);
            }
            Err(error) => last = Some(error),
        }
    }
    bail!(
        "TCP connect to {} failed: {:?}",
        authority(host, port),
        last
    )
}

fn no_proxy_matches(list: &str, host: &str, port: u16) -> bool {
    let host = host.to_ascii_lowercase();
    list.split(',').any(|entry| {
        let entry = entry.trim().to_ascii_lowercase();
        if entry == "*" {
            return true;
        }
        let (name, required_port) = if let Some(rest) = entry.strip_prefix('[') {
            let Some((name, suffix)) = rest.split_once(']') else {
                return false;
            };
            (
                name,
                suffix.strip_prefix(':').and_then(|s| s.parse::<u16>().ok()),
            )
        } else if entry.matches(':').count() == 1 {
            let (name, port) = entry.split_once(':').unwrap();
            (name, port.parse::<u16>().ok())
        } else {
            (entry.as_str(), None)
        };
        let name = name.trim_start_matches('.');
        !name.is_empty()
            && required_port.is_none_or(|p| p == port)
            && (host == name
                || host
                    .strip_suffix(name)
                    .is_some_and(|prefix| prefix.ends_with('.')))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ipv6_and_redacts_credentials() {
        let target = PoolAddress::parse("stratum+ssl://[::1]:443").unwrap();
        assert_eq!(target.authority(), "[::1]:443");
        let proxy = Proxy::parse("socks5h://user:p%40ss@[::1]:1080").unwrap();
        assert_eq!(proxy.pass, "p@ss");
        assert!(!format!("{proxy:?}").contains("user"));
        assert!(Proxy::parse("https://localhost:8080").is_err());
        assert!(PoolAddress::parse("stratum+ssl://localhost:443/path").is_err());
        assert_eq!(Proxy::parse("http://localhost:80").unwrap().port, 80);
        assert_eq!(Proxy::parse("http://localhost").unwrap().port, 8080);
    }
    #[test]
    fn no_proxy_respects_domain_boundaries_and_ports() {
        assert!(no_proxy_matches(
            ".example.com:443",
            "pool.example.com",
            443
        ));
        assert!(!no_proxy_matches("example.com", "notexample.com", 443));
        assert!(!no_proxy_matches("example.com:80", "example.com", 443));
        assert!(no_proxy_matches("[::1]:443", "::1", 443));
    }

    #[test]
    fn http_connect_auth_preserves_tunneled_bytes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            let header = String::from_utf8(header).unwrap();
            assert!(header.starts_with("CONNECT [::1]:443 HTTP/1.1\r\n"));
            assert!(header.contains("Proxy-Authorization: Basic dXNlcjpwYXNz\r\n"));
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\njob-bytes")
                .unwrap();
        });
        let proxy = Proxy::parse(&format!("http://user:pass@{address}")).unwrap();
        let target = PoolAddress::parse("stratum+ssl://[::1]:443").unwrap();
        let mut stream = connect(&target, Some(&proxy)).unwrap();
        let mut data = [0; 9];
        stream.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"job-bytes");
        server.join().unwrap();
    }

    #[test]
    fn socks_auth_and_remote_dns_do_not_resolve_locally() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut methods = [0; 4];
            stream.read_exact(&mut methods).unwrap();
            assert_eq!(methods, [5, 2, 0, 2]);
            stream.write_all(&[5, 2]).unwrap();
            let mut auth = [0; 11];
            stream.read_exact(&mut auth).unwrap();
            assert_eq!(&auth, b"\x01\x04user\x04pass");
            stream.write_all(&[1, 0]).unwrap();
            let mut request = [0; 5];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(&request[..4], &[5, 1, 0, 3]);
            let mut hostname = vec![0; request[4] as usize];
            stream.read_exact(&mut hostname).unwrap();
            assert_eq!(hostname, b"pool.invalid");
            let mut port = [0; 2];
            stream.read_exact(&mut port).unwrap();
            assert_eq!(u16::from_be_bytes(port), 443);
            stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
        });
        let proxy = Proxy::parse(&format!("socks5h://user:pass@{address}")).unwrap();
        let target = PoolAddress::parse("stratum+ssl://pool.invalid:443").unwrap();
        connect(&target, Some(&proxy)).unwrap();
        server.join().unwrap();
    }
}
