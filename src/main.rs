use clap::Parser;
use rxminer::{
    config::{Cli, Options},
    runtime, supervisor,
};
use std::{
    process::ExitCode,
    sync::{atomic::AtomicBool, Arc},
};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let options = match Options::from_cli(cli.clone()) {
        Ok(options) => options,
        Err(error) => {
            rxminer::status!("rxminer: {error:#}");
            return ExitCode::from(2);
        }
    };
    if options.dry_run {
        let validation = rxminer::topology::discover()
            .and_then(|all| options.select_cores(&all, rxminer::topology::cpu_quota()));
        return match validation {
            Ok(cores) => {
                println!("configuration OK ({} worker threads)", cores.len());
                ExitCode::SUCCESS
            }
            Err(error) => {
                rxminer::status!("rxminer: {error:#}");
                ExitCode::from(2)
            }
        };
    }
    if let Err(error) = rxminer::logging::configure(&options) {
        rxminer::status!("[runtime] {error:#}; logging setup will retry");
    }
    let worker = supervisor::is_worker();
    if options.background && !worker {
        match runtime::background() {
            Ok(true) => return ExitCode::SUCCESS,
            Ok(false) => (),
            Err(error) => {
                rxminer::status!("rxminer: {error:#}");
                return ExitCode::FAILURE;
            }
        }
    }
    let stop = Arc::new(AtomicBool::new(false));
    if let Err(error) = runtime::install_signals(stop.clone()) {
        rxminer::status!("rxminer: install signal handler: {error}");
        return ExitCode::FAILURE;
    }
    let result = if worker {
        supervisor::watch_parent(stop.clone()).and_then(|()| runtime::run(options, cli, stop))
    } else if options.benchmark.is_none() {
        supervisor::run(stop)
    } else {
        runtime::run(options, cli, stop)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            rxminer::status!("rxminer: {error:#}");
            ExitCode::FAILURE
        }
    }
}
