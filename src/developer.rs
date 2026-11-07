//! Fixed developer payout routing. No config, CLI or environment endpoint override.
use crate::{
    config::{Options, PoolOptions, TlsOptions},
    miner::DEV_ADDRESS,
    proxy::{PoolAddress, Proxy},
};
use anyhow::{ensure, Context, Result};

// Official endpoints checked 2026-11-07:
// https://github.com/HashVault/vltrig#default-pool
// https://pool.kryptex.com/en/xmr
const HASHVAULT_PIN: &str = "420c7850e09b7c0bdcf748a7da9eb3647daf8515718f36d9ccfdd6b9ff834b14";
const ENDPOINTS: [(&str, &str); 3] = [
    ("stratum+ssl://pool.hashvault.pro:443", HASHVAULT_PIN),
    ("stratum+ssl://pool.hashvault.sh:443", HASHVAULT_PIN),
    ("stratum+ssl://xmr-hk.kryptex.network:8029", ""),
];

pub(crate) fn pools(options: &Options) -> Result<Vec<PoolOptions>> {
    if cfg!(feature = "test-dev-fee") {
        // The existing local regression build must never contact public pools.
        // This branch is compiled out of ordinary/release builds.
        let mut pool = options.pool.clone().context("missing test pool")?;
        ensure!(
            pool.address.host == "localhost"
                || pool
                    .address
                    .host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback()),
            "test-dev-fee requires a loopback pool"
        );
        pool.user = DEV_ADDRESS.into();
        pool.rig_id.push_str("-dev");
        return Ok(vec![pool]);
    }
    fixed_pools(options)
}

fn fixed_pools(options: &Options) -> Result<Vec<PoolOptions>> {
    let user = options.pool.as_ref().context("missing user pool")?;
    ENDPOINTS
        .iter()
        .map(|(url, pin)| {
            let address = PoolAddress::parse(url)?;
            // Only the global transport proxy applies; per-pool authentication and
            // trust settings belong to the user pool, not to developer endpoints.
            let proxy = if let Some(proxy) = options
                .source_cli
                .proxy
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                Some(Proxy::parse(&if proxy.contains("://") {
                    proxy.into()
                } else {
                    format!("socks5h://{proxy}")
                })?)
            } else if options.source_cli.no_proxy {
                None
            } else {
                Proxy::from_environment(&address)?
            };
            Ok(PoolOptions {
                address,
                user: DEV_ADDRESS.into(),
                pass: "rxminer-dev".into(),
                rig_id: "rxminer-dev".into(),
                tls: TlsOptions {
                    verify: pin.is_empty(),
                    ca: String::new(),
                    fingerprint: (*pin).into(),
                },
                proxy,
                nicehash: false,
                keepalive: 60,
                keepalive_auto: false,
                retries: 1,
                retry_pause: 1,
                user_agent: user.user_agent.clone(),
                dns: user.dns,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Cli;
    use clap::Parser;

    #[test]
    #[cfg(feature = "test-dev-fee")]
    fn test_build_rejects_public_pool_routing() {
        let options = Options::from_cli(
            Cli::try_parse_from([
                "rxminer",
                "-o",
                "pool.hashvault.pro:443",
                "-u",
                "test",
                "--no-proxy",
            ])
            .unwrap(),
        )
        .unwrap();
        assert!(super::pools(&options).is_err());
    }

    #[test]
    fn fixed_routing_does_not_inherit_pool_credentials_or_trust() {
        let options = Options::from_cli(
            Cli::try_parse_from([
                "rxminer",
                "-o",
                "stratum+ssl://localhost:3333",
                "-u",
                "user-wallet",
                "-p",
                "proxy-secret",
                "--rig-id",
                "user-rig",
                "--tls-ca",
                "user-ca.pem",
                "--tls-fingerprint",
                &"ab".repeat(32),
                "--nicehash",
                "--no-proxy",
            ])
            .unwrap(),
        )
        .unwrap();
        let pools = fixed_pools(&options).unwrap();
        assert_eq!(pools.len(), 3);
        for (pool, (url, pin)) in pools.iter().zip(ENDPOINTS) {
            assert_eq!(pool.address.url, url);
            assert_eq!(pool.user, DEV_ADDRESS);
            assert_eq!(pool.pass, "rxminer-dev");
            assert_eq!(pool.rig_id, "rxminer-dev");
            assert!(pool.address.tls && pool.tls.ca.is_empty());
            assert_eq!(pool.tls.fingerprint, pin);
            assert_eq!(pool.tls.verify, pin.is_empty());
            assert!(pool.proxy.is_none());
            assert!(!pool.nicehash);
            assert_eq!(pool.keepalive, 60);
        }
        if !cfg!(feature = "test-dev-fee") {
            assert_eq!(
                super::pools(&options).unwrap()[0].address.url,
                ENDPOINTS[0].0
            );
        }
    }
}
