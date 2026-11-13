#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
//! Local fixtures only; enable test-dev-fee and run ignored tests sequentially.
#![cfg(feature = "test-dev-fee")]
#[path = "support/network.rs"]
mod network;
mod support;

use anyhow::{ensure, Result};
use network::{Api, Certificate, Pool, Proxy};
use serde_json::{json, Value};
use std::{collections::HashSet, fs, path::Path, thread, time::Duration};
use support::{miner_command, port, test_cpus, wait_until, write_json, Process, TempDir};

fn binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_rxminer"))
}
fn shares_ready(data: &Value, threads: u64) -> bool {
    data["threads"] == threads
        && data["shares"]["accepted"].as_u64().unwrap_or(0) > 0
        && data["dev_shares"]["accepted"].as_u64().unwrap_or(0) > 0
}

#[test]
#[ignore = "TLS/proxy/share regression; full-memory execution is covered by portable tests"]
fn protocol_and_fee_windows() -> Result<()> {
    let temp = TempDir::new("protocol")?;
    let cert = Certificate::new(&temp.0)?;
    for mode in ["direct-tls", "http-tls", "socks5h-tls", "plain"] {
        let tls = mode != "plain";
        let mut pool = Pool::new(tls.then_some(&cert), false, 0)?;
        let proxy = if matches!(mode, "http-tls" | "socks5h-tls") {
            Some(Proxy::new(mode == "socks5h-tls", pool.port())?)
        } else {
            None
        };
        let api = Api {
            port: port()?,
            token: "integration-token".into(),
            certificate: None,
        };
        let mut command = miner_command(binary())?;
        if mode == "direct-tls" {
            let config = temp.0.join("config.json");
            let cpus: Vec<_> = test_cpus()?.into_iter().cycle().take(2).collect();
            write_json(
                &config,
                &json!({
                    "pools": [{"url": format!("127.0.0.1:{}", pool.port()), "user": "test-wallet", "tls": true,
                        "tls-ca": cert.cert, "tls-fingerprint": cert.fingerprint}],
                    "cpu": {"rx": cpus, "huge-pages": false},
                    "randomx": {"numa": false, "init": 2, "mode": "light"},
                    "http": {"enabled": true, "host": "127.0.0.1", "port": api.port,
                        "access-token": api.token, "restricted": true}, "print-time": 0
                }),
            )?;
            command.arg("-c").arg(config).arg("--no-proxy");
        } else {
            let scheme = if tls { "stratum+ssl" } else { "stratum+tcp" };
            command
                .arg("-o")
                .arg(format!("{scheme}://127.0.0.1:{}", pool.port()))
                .args([
                    "-u",
                    "test-wallet",
                    "-t",
                    "2",
                    "--randomx-mode",
                    "light",
                    "--randomx-no-numa",
                    "--no-huge-pages",
                    "--no-proxy",
                    "--http-port",
                ])
                .arg(api.port.to_string())
                .arg("--http-access-token")
                .arg(&api.token);
            if tls {
                command
                    .arg("--tls-ca")
                    .arg(&cert.cert)
                    .arg("--tls-fingerprint")
                    .arg(&cert.fingerprint);
            }
            if let Some(proxy) = &proxy {
                let prefix = if mode == "http-tls" { "http://" } else { "" };
                command
                    .arg("--proxy")
                    .arg(format!("{prefix}127.0.0.1:{}", proxy.port()));
            }
        }
        let mut miner = Process::spawn(&mut command, temp.0.join(format!("{mode}.log")))?;
        let data = api.wait_for(&mut miner, 90, |data| shares_ready(data, 2))?;
        assert_eq!(data["nodes"], 1);
        assert_eq!(data["dev_fee"]["percent"], "<1%");
        assert_eq!(api.request("GET", "/summary", None, false)?.0, 401);
        if mode == "direct-tls" {
            thread::sleep(Duration::from_secs(6));
            miner.ensure_running()?;
            for path in ["/", "/hashrates", "/shares"] {
                assert!(api.get(path)?.is_object());
            }
            assert_eq!(api.request("GET", "/missing", None, true)?.0, 404);
            let shares = pool.snapshot().shares;
            let wallets: HashSet<_> = shares.iter().map(|(wallet, _)| wallet.as_str()).collect();
            assert_eq!(
                wallets,
                HashSet::from(["test-wallet", data["dev_fee"]["address"].as_str().unwrap()])
            );
            // Independent connections have no guaranteed arrival order. Check
            // both wallets and nonce continuity across developer fee windows.
            let unique: HashSet<_> = shares.iter().collect();
            assert_eq!(
                unique.len(),
                shares.len(),
                "duplicate nonce across fee windows"
            );
        }
        if mode == "plain" {
            pool.stop();
            assert!(
                !miner.wait(10)?.success(),
                "mandatory developer disconnect must be fatal"
            );
        } else {
            miner.signal(libc::SIGINT)?;
            miner.success(15)?;
        }
        println!(
            "PASS {mode}: user={} dev={}",
            data["shares"]["accepted"], data["dev_shares"]["accepted"]
        );
    }
    Ok(())
}

#[test]
#[ignore = "local TLS and mandatory developer-login rejection"]
fn rejected_connections() -> Result<()> {
    let temp = TempDir::new("rejections")?;
    let cert = Certificate::new(&temp.0)?;
    for name in ["untrusted-cert", "wrong-pin", "denied-dev"] {
        let pool = Pool::new(Some(&cert), name == "denied-dev", 0)?;
        let mut command = miner_command(binary())?;
        command
            .arg("-o")
            .arg(format!("stratum+ssl://127.0.0.1:{}", pool.port()))
            .args(["-u", "test-wallet", "-t", "2", "--no-proxy"]);
        match name {
            "untrusted-cert" => {
                command.arg("--tls-verify");
            }
            "wrong-pin" => {
                command.arg("--tls-fingerprint").arg("00".repeat(32));
            }
            _ => (),
        }
        let mut miner = Process::spawn(&mut command, temp.0.join(format!("{name}.log")))?;
        assert!(!miner.wait(25)?.success());
        assert!(
            miner
                .output()
                .contains("required pool connection unavailable"),
            "{}",
            miner.output()
        );
        assert!(!miner.output().contains("developer"), "{}", miner.output());
        println!("PASS {name}");
    }
    Ok(())
}

fn rpc(api: &Api, method: &str) -> Result<Value> {
    let (status, body) = api.request(
        "POST",
        "/json_rpc",
        Some(&json!({"id": 1, "method": method})),
        true,
    )?;
    ensure!(status == 200, "RPC failed: {status} {body}");
    Ok(body)
}

#[test]
#[ignore = "local failover, NiceHash, reload, authenticated TLS API and background regression"]
fn runtime_controls() -> Result<()> {
    let temp = TempDir::new("runtime")?;
    let cert = Certificate::new(&temp.0)?;
    let pool = Pool::new(None, false, 0xabfffffe)?;
    let api = Api {
        port: port()?,
        token: "local-token".into(),
        certificate: Some(cert.cert.clone()),
    };
    let config = temp.0.join("config.json");
    let configured_log = temp.0.join("configured.log");
    let pools = [port()?, pool.port()].map(|port| {
        json!({"url": format!("127.0.0.1:{port}"),
        "user": "runtime-test", "nicehash": true, "keepalive": 1})
    });
    let mut data = json!({
        "pools": pools,
        "randomx": {"mode": "light", "numa": false, "wrmsr": false, "scratchpad_prefetch_mode": 2},
        "cpu": {"rx": [-1], "huge-pages": false, "memory-pool": true, "priority": 1},
        "http": {"enabled": true, "port": api.port, "access-token": api.token, "restricted": false},
        "tls": {"enabled": true, "cert": cert.cert, "cert_key": cert.key, "protocols": "TLSv1.2 TLSv1.3"},
        "retries": 2, "retry-pause": 0, "user-agent": "runtime-agent", "print-time": 0,
        "health-print-time": 1, "autosave": false, "watch": true, "log-file": configured_log,
        "dns": {"ip_version": 4, "ttl": 1}
    });
    write_json(&config, &data)?;
    let mut command = miner_command(binary())?;
    command.arg("-c").arg(&config).arg("--no-proxy");
    let mut miner = Process::spawn(&mut command, temp.0.join("miner.log"))?;
    api.wait_for(&mut miner, 30, |v| shares_ready(v, 1))?;
    assert!(miner.output().contains("switching to pool 2"));
    wait_until(5, || Ok((pool.snapshot().keepalives > 0).then_some(())))?;
    assert!(pool
        .snapshot()
        .agents
        .iter()
        .any(|agent| agent.contains("runtime-agent")));
    wait_until(5, || {
        Ok(fs::read_to_string(&configured_log)
            .unwrap_or_default()
            .contains("[health]")
            .then_some(()))
    })?;
    assert!(pool
        .snapshot()
        .shares
        .iter()
        .all(|(_, nonce)| nonce.ends_with("ab")));
    println!("PASS failover, NiceHash wrap, keepalive, user-agent, light mining, logging");

    assert_eq!(
        api.request(
            "POST",
            "/json_rpc",
            Some(&json!({"id":1, "method":"stop"})),
            false
        )?
        .0,
        401
    );
    assert_eq!(api.request("GET", "/1/config", None, false)?.0, 401);
    let before = fs::read(&config)?;
    assert_eq!(
        api.request("PUT", "/1/config", Some(&json!({"cpu":{"rx":[-9]}})), true)?
            .0,
        400
    );
    assert_eq!(fs::read(&config)?, before);
    assert_eq!(rpc(&api, "pause")?["result"]["status"], "OK");
    thread::sleep(Duration::from_millis(500));
    let frozen = api.get("/summary")?["hashes"].as_u64().unwrap();
    thread::sleep(Duration::from_millis(500));
    assert_eq!(api.get("/summary")?["hashes"], frozen);
    let paused = api.get("/2/summary")?;
    assert_eq!(paused["paused"], true);
    let rates: Vec<_> = paused["hashrate"]["total"]
        .as_array()
        .unwrap()
        .iter()
        .map(Value::as_f64)
        .collect();
    assert_eq!(rates, vec![Some(0.0); 3]);
    rpc(&api, "resume")?;
    api.wait_for(&mut miner, 5, |v| {
        v["hashes"].as_u64().unwrap_or(0) > frozen
    })?;
    println!("PASS authenticated HTTPS API, invalid PUT, pause/resume");

    fs::write(&config, "{ invalid JSON")?;
    wait_until(5, || {
        miner.ensure_running()?;
        Ok(miner.output().contains("reload rejected").then_some(()))
    })?;
    assert!(shares_ready(&api.get("/summary")?, 1));
    data["cpu"]["rx"] = json!([-1, -1]);
    data["watch"] = json!(false);
    write_json(&config, &data)?;
    api.wait_for(&mut miner, 30, |v| shares_ready(v, 2))?;
    assert_eq!(api.get("/1/config")?["cpu"]["rx"], json!([-1, -1]));
    data["cpu"]["rx"] = json!([-1]);
    assert_eq!(api.request("PUT", "/1/config", Some(&data), true)?.0, 200);
    api.wait_for(&mut miner, 30, |v| shares_ready(v, 1))?;
    rpc(&api, "stop")?;
    miner.success(10)?;
    println!("PASS invalid reload, worker replacement, API reload with watch disabled, stop");

    let generated = temp.0.join("generated.json");
    let background_log = temp.0.join("background.log");
    write_json(
        &generated,
        &json!({
            "cpu": {"max-threads-hint": 1, "huge-pages": false},
            "randomx": {"mode": "light", "numa": false, "wrmsr": false},
            "benchmark": {"seconds": 1}, "background": true,
            "log-file": background_log, "autosave": true
        }),
    )?;
    let mut command = miner_command(binary())?;
    command.arg("-c").arg(&generated);
    let mut background = Process::spawn(&mut command, temp.0.join("background-parent.log"))?;
    background.success(10)?;
    wait_until(30, || {
        Ok(fs::read_to_string(&background_log)
            .unwrap_or_default()
            .contains(" [final]")
            .then_some(()))
    })?;
    let saved: Value = serde_json::from_slice(&fs::read(generated)?)?;
    assert_eq!(saved["cpu"]["rx"].as_array().unwrap().len(), 1);
    assert!(!fs::read_to_string(background_log)?.contains("restarting workers"));
    println!("PASS background completion and automatic profile persistence");
    Ok(())
}
