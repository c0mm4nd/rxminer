//! Shared TLS policy. Trust and pinning are checked before application data.
use anyhow::{ensure, Context, Result};
use rustls::{
    client::{
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        WebPkiServerVerifier,
    },
    crypto::{self, CryptoProvider},
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, RootCertStore, ServerConfig, SignatureScheme,
    SupportedCipherSuite,
};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

pub(crate) fn certificates(path: impl AsRef<Path>) -> Result<Vec<CertificateDer<'static>>> {
    let certs = CertificateDer::pem_file_iter(path)?.collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(!certs.is_empty(), "PEM file contains no certificates");
    Ok(certs)
}

#[derive(Debug)]
struct PoolVerifier {
    trust: Option<Arc<WebPkiServerVerifier>>,
    pin: Option<[u8; 32]>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PoolVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if let Some(trust) = &self.trust {
            trust.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)?;
        }
        if let Some(pin) = &self.pin {
            use subtle::ConstantTimeEq;
            let actual = Sha256::digest(end_entity.as_ref());
            if !bool::from(actual.as_slice().ct_eq(pin)) {
                return Err(rustls::Error::General(
                    "TLS certificate fingerprint mismatch".into(),
                ));
            }
        }
        // Unverified mode and pin-only mode deliberately skip CA/hostname
        // validation, as configured. Both still verify handshake signatures.
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub(crate) fn client_config(options: &crate::config::TlsOptions) -> Result<Arc<ClientConfig>> {
    let provider = Arc::new(crypto::ring::default_provider());
    let trust = if options.verify || !options.ca.is_empty() {
        let native = rustls_native_certs::load_native_certs();
        let mut roots = RootCertStore::empty();
        roots.add_parsable_certificates(native.certs);
        if !options.ca.is_empty() {
            for cert in certificates(&options.ca).context("load TLS CA")? {
                roots.add(cert).context("invalid TLS CA certificate")?;
            }
        }
        ensure!(
            !roots.is_empty(),
            "no usable TLS root certificates: {:?}",
            native.errors
        );
        Some(
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                .build()?,
        )
    } else {
        None
    };
    let pin = if options.fingerprint.is_empty() {
        None
    } else {
        let mut bytes = [0; 32];
        hex::decode_to_slice(&options.fingerprint, &mut bytes)
            .context("invalid TLS fingerprint")?;
        Some(bytes)
    };
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PoolVerifier {
            trust,
            pin,
            provider,
        }))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

// Resume the same handshake after partial records/WouldBlock. A nonblocking
// socket lets the outer loop enforce cancellation and a total deadline.
pub(crate) fn handshake(
    mut advance: impl FnMut() -> io::Result<bool>,
    stopped: impl Fn() -> bool,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        ensure!(!stopped(), "TLS handshake interrupted");
        ensure!(Instant::now() < deadline, "TLS handshake timed out");
        match advance() {
            Ok(true) => return Ok(()),
            Ok(false) => (),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error).context("TLS handshake"),
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn cipher_alias(suite: SupportedCipherSuite) -> &'static str {
    use rustls::CipherSuite::*;
    match suite.suite() {
        TLS13_AES_128_GCM_SHA256 => "TLS_AES_128_GCM_SHA256",
        TLS13_AES_256_GCM_SHA384 => "TLS_AES_256_GCM_SHA384",
        TLS13_CHACHA20_POLY1305_SHA256 => "TLS_CHACHA20_POLY1305_SHA256",
        TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 => "ECDHE-ECDSA-AES128-GCM-SHA256",
        TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384 => "ECDHE-ECDSA-AES256-GCM-SHA384",
        TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256 => "ECDHE-ECDSA-CHACHA20-POLY1305",
        TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 => "ECDHE-RSA-AES128-GCM-SHA256",
        TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 => "ECDHE-RSA-AES256-GCM-SHA384",
        TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256 => "ECDHE-RSA-CHACHA20-POLY1305",
        _ => "",
    }
}

fn cipher_list(
    provider: &CryptoProvider,
    names: Option<&str>,
    version: rustls::ProtocolVersion,
) -> Result<Vec<SupportedCipherSuite>> {
    let available: Vec<_> = provider
        .cipher_suites
        .iter()
        .copied()
        .filter(|suite| suite.version().version == version)
        .collect();
    let Some(names) = names else {
        return Ok(available);
    };
    let mut selected = Vec::new();
    for name in names.split(':') {
        let suite = available.iter().find(|suite| {
            format!("{:?}", suite.suite()) == name || (!name.is_empty() && cipher_alias(**suite) == name)
        }).with_context(|| format!("unsupported API TLS cipher {name:?}; use explicit supported IANA or ECDHE cipher names, not OpenSSL expressions"))?;
        if !selected.contains(suite) {
            selected.push(*suite);
        }
    }
    Ok(selected)
}

pub(crate) fn server_config(
    options: &crate::http::TlsOptions,
) -> Result<Option<Arc<ServerConfig>>> {
    if !options.enabled {
        return Ok(None);
    }
    ensure!(
        options.dhparam.is_none(),
        "tls.dhparam is unsupported by rustls; remove it to use ECDHE key exchange"
    );
    let mut versions = Vec::new();
    for name in options
        .protocols
        .as_deref()
        .unwrap_or("TLSv1.2 TLSv1.3")
        .split_whitespace()
    {
        let version = match name {
            "TLSv1.2" => &rustls::version::TLS12,
            "TLSv1.3" => &rustls::version::TLS13,
            _ => anyhow::bail!("API TLS protocols must be TLSv1.2 and/or TLSv1.3"),
        };
        if !versions.contains(&version) {
            versions.push(version);
        }
    }
    ensure!(
        !versions.is_empty(),
        "API TLS protocols must be TLSv1.2 and/or TLSv1.3"
    );
    let mut provider = crypto::ring::default_provider();
    let mut suites = cipher_list(
        &provider,
        options.ciphersuites.as_deref(),
        rustls::ProtocolVersion::TLSv1_3,
    )?;
    suites.extend(cipher_list(
        &provider,
        options.ciphers.as_deref(),
        rustls::ProtocolVersion::TLSv1_2,
    )?);
    provider.cipher_suites = suites;
    let config = ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&versions)?
        .with_no_client_auth()
        .with_single_cert(
            certificates(options.cert.as_deref().context("tls.cert is required")?)
                .context("load API TLS certificate chain")?,
            PrivateKeyDer::from_pem_file(
                options
                    .cert_key
                    .as_deref()
                    .context("tls.cert_key is required")?,
            )
            .context("load API TLS private key")?,
        )
        .context("invalid API TLS certificate or private key")?;
    Ok(Some(Arc::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::{ClientConnection, ServerConnection, StreamOwned};
    use std::{
        fs,
        io::{Read, Write},
        net::TcpStream,
        path::PathBuf,
        sync::atomic::{AtomicBool, AtomicU64, Ordering},
    };

    struct Fixture {
        dir: PathBuf,
        cert: rcgen::Certificate,
        options: crate::http::TlsOptions,
    }
    impl Fixture {
        fn new(expired: bool) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let dir = loop {
                let dir = std::env::temp_dir().join(format!(
                    "rxminer-tls-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&dir) {
                    Ok(()) => break dir,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create TLS fixture: {error}"),
                }
            };
            let key = rcgen::KeyPair::generate().unwrap();
            let mut params =
                rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])
                    .unwrap();
            if expired {
                params.not_before = rcgen::date_time_ymd(2000, 1, 1);
                params.not_after = rcgen::date_time_ymd(2001, 1, 1);
            }
            let cert = params.self_signed(&key).unwrap();
            fs::write(dir.join("cert.pem"), cert.pem()).unwrap();
            fs::write(dir.join("key.pem"), key.serialize_pem()).unwrap();
            let options = crate::http::TlsOptions {
                enabled: true,
                cert: Some(dir.join("cert.pem").to_str().unwrap().into()),
                cert_key: Some(dir.join("key.pem").to_str().unwrap().into()),
                ..Default::default()
            };
            Self { dir, cert, options }
        }

        fn client(&self, verify: bool, pin: String) -> crate::config::TlsOptions {
            crate::config::TlsOptions {
                verify,
                ca: if verify {
                    self.options.cert.clone().unwrap()
                } else {
                    String::new()
                },
                fingerprint: pin,
            }
        }

        fn pin(&self) -> String {
            hex::encode(Sha256::digest(self.cert.der().as_ref()))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    // Fragment every TLS flight to exercise record reassembly without network
    // timing. Verify actual encrypted application data after the handshake.
    fn exchange(
        server: Arc<ServerConfig>,
        client: Arc<ClientConfig>,
        host: &str,
    ) -> Result<rustls::ProtocolVersion> {
        let mut client = ClientConnection::new(client, ServerName::try_from(host.to_owned())?)?;
        let mut server = ServerConnection::new(server)?;
        for _ in 0..32 {
            let mut flight = Vec::new();
            client.write_tls(&mut flight)?;
            for chunk in flight.chunks(7) {
                server.read_tls(&mut &chunk[..])?;
                server.process_new_packets()?;
            }
            flight.clear();
            server.write_tls(&mut flight)?;
            for chunk in flight.chunks(11) {
                client.read_tls(&mut &chunk[..])?;
                client.process_new_packets()?;
            }
            if !client.is_handshaking() && !server.is_handshaking() {
                client.writer().write_all(b"share")?;
                flight.clear();
                client.write_tls(&mut flight)?;
                server.read_tls(&mut flight.as_slice())?;
                server.process_new_packets()?;
                let mut received = [0; 5];
                server.reader().read_exact(&mut received)?;
                ensure!(&received == b"share", "TLS application data mismatch");
                return client.protocol_version().context("missing TLS version");
            }
        }
        anyhow::bail!("TLS handshake made no progress")
    }

    #[test]
    fn trust_hostname_and_pin_policies_for_both_versions() {
        let fixture = Fixture::new(false);
        let untrusted = Fixture::new(false);
        for (protocol, version) in [
            ("TLSv1.2", rustls::ProtocolVersion::TLSv1_2),
            ("TLSv1.3", rustls::ProtocolVersion::TLSv1_3),
        ] {
            let mut options = fixture.options.clone();
            options.protocols = Some(protocol.into());
            options.ciphers = Some("ECDHE-ECDSA-AES128-GCM-SHA256".into());
            options.ciphersuites = Some("TLS_AES_128_GCM_SHA256".into());
            let server = server_config(&options).unwrap().unwrap();
            for (client, host, pass) in [
                (fixture.client(false, String::new()), "localhost", true),
                (fixture.client(true, String::new()), "localhost", true),
                (fixture.client(true, String::new()), "127.0.0.1", true),
                (fixture.client(true, String::new()), "wrong.example", false),
                (fixture.client(false, fixture.pin()), "wrong.example", true),
                (fixture.client(false, "00".repeat(32)), "localhost", false),
                (fixture.client(true, fixture.pin()), "localhost", true),
                (fixture.client(true, "00".repeat(32)), "localhost", false),
                (untrusted.client(true, fixture.pin()), "localhost", false),
            ] {
                let result = exchange(server.clone(), client_config(&client).unwrap(), host);
                assert_eq!(
                    result.is_ok(),
                    pass,
                    "{protocol} verify={} host={host} pin={} result={result:?}",
                    client.verify,
                    !client.fingerprint.is_empty()
                );
                if pass {
                    assert_eq!(result.unwrap(), version);
                }
            }
        }
    }

    #[test]
    fn pin_only_and_unverified_modes_still_reject_invalid_handshake_signatures() {
        #[derive(Debug)]
        struct WrongKey(Arc<rustls::sign::CertifiedKey>);
        impl rustls::server::ResolvesServerCert for WrongKey {
            fn resolve(
                &self,
                _: rustls::server::ClientHello<'_>,
            ) -> Option<Arc<rustls::sign::CertifiedKey>> {
                Some(self.0.clone())
            }
        }
        let fixture = Fixture::new(false);
        let other = Fixture::new(false);
        let provider = Arc::new(crypto::ring::default_provider());
        let key = provider
            .key_provider
            .load_private_key(
                PrivateKeyDer::from_pem_file(other.options.cert_key.as_ref().unwrap()).unwrap(),
            )
            .unwrap();
        let wrong = Arc::new(WrongKey(Arc::new(rustls::sign::CertifiedKey::new(
            vec![fixture.cert.der().clone()],
            key,
        ))));
        for version in [&rustls::version::TLS12, &rustls::version::TLS13] {
            let server = Arc::new(
                ServerConfig::builder_with_provider(provider.clone())
                    .with_protocol_versions(&[version])
                    .unwrap()
                    .with_no_client_auth()
                    .with_cert_resolver(wrong.clone()),
            );
            for pin in [String::new(), fixture.pin()] {
                let error = exchange(
                    server.clone(),
                    client_config(&fixture.client(false, pin)).unwrap(),
                    "localhost",
                )
                .unwrap_err();
                assert!(
                    matches!(
                        error.downcast_ref::<rustls::Error>(),
                        Some(rustls::Error::InvalidCertificate(
                            rustls::CertificateError::BadSignature
                        ))
                    ),
                    "{error:?}"
                );
            }
        }
    }

    #[test]
    fn expired_certificates_fail_trust_even_with_a_matching_pin() {
        let fixture = Fixture::new(true);
        let server = server_config(&fixture.options).unwrap().unwrap();
        let client = client_config(&fixture.client(true, fixture.pin())).unwrap();
        let error = exchange(server, client, "localhost").unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<rustls::Error>(),
                Some(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::Expired
                        | rustls::CertificateError::ExpiredContext { .. }
                ))
            ),
            "{error:?}"
        );
    }

    #[test]
    fn invalid_tls_configuration_fails_before_listening() {
        let fixture = Fixture::new(false);
        let other = Fixture::new(false);
        for (field, value) in [
            ("protocols", "TLSv1.1"),
            ("protocols", ""),
            ("ciphers", "HIGH:!aNULL"),
            ("ciphersuites", "TLS_FAKE"),
            ("dhparam", "dh.pem"),
            ("cert_key", other.options.cert_key.as_deref().unwrap()),
        ] {
            let mut options = fixture.options.clone();
            match field {
                "protocols" => options.protocols = Some(value.into()),
                "ciphers" => options.ciphers = Some(value.into()),
                "ciphersuites" => options.ciphersuites = Some(value.into()),
                "dhparam" => options.dhparam = Some(value.into()),
                "cert_key" => options.cert_key = Some(value.into()),
                _ => unreachable!(),
            }
            assert!(server_config(&options).is_err(), "{field}={value}");
        }
        fs::write(fixture.dir.join("empty.pem"), "not a certificate").unwrap();
        let mut client = fixture.client(true, String::new());
        client.ca = fixture.dir.join("empty.pem").to_str().unwrap().into();
        assert!(client_config(&client).is_err());
        client.ca = fixture.dir.join("missing.pem").to_str().unwrap().into();
        assert!(client_config(&client).is_err());
        assert!(client_config(&fixture.client(false, "00".into())).is_err());
    }

    #[test]
    fn https_authentication_and_stalled_handshake_shutdown() {
        let fixture = Fixture::new(false);
        // macOS system certificate loading can exceed the server's handshake
        // deadline on a busy runner. Finish client setup before opening a socket.
        let client = client_config(&fixture.client(true, fixture.pin())).unwrap();
        let control = crate::control::Control::new(Arc::new(AtomicBool::new(false)));
        let server = crate::http::Server::start_tls(
            "127.0.0.1",
            0,
            Some("secret".into()),
            server_config(&fixture.options).unwrap(),
            Arc::new(|_, _, _| (200, serde_json::json!({"ok":true}))),
            control.clone(),
        )
        .unwrap();
        for (token, status) in [("secret", "200"), ("wrong!", "401")] {
            let socket = TcpStream::connect(server.address).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let conn =
                ClientConnection::new(client.clone(), "localhost".try_into().unwrap()).unwrap();
            let mut tls = StreamOwned::new(conn, socket);
            write!(
                tls,
                "GET /summary HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\n\r\n"
            )
            .unwrap();
            tls.flush().unwrap();
            let mut response = String::new();
            tls.read_to_string(&mut response).unwrap();
            assert!(
                response.starts_with(&format!("HTTP/1.1 {status}")),
                "{response}"
            );
        }
        let _stalled = TcpStream::connect(server.address).unwrap();
        thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        drop(server);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(!control.stopped());
    }
}
