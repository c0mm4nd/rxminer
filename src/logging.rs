use crate::config::Options;
use anyhow::{Context, Result};
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{IsTerminal, Write},
    os::unix::net::UnixDatagram,
    sync::{Mutex, OnceLock},
};

#[derive(Default)]
struct Logger {
    file: Option<File>,
    syslog: Option<UnixDatagram>,
    colors: bool,
}
static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

pub fn configure(options: &Options) -> Result<()> {
    let file = options
        .log_file
        .as_ref()
        .map(|path| {
            OpenOptions::new()
                .append(true)
                .create(true)
                .open(path)
                .with_context(|| format!("open log {}", path.display()))
        })
        .transpose()?;
    let syslog = if options.syslog {
        let socket = UnixDatagram::unbound()?;
        socket.connect("/dev/log").context("connect syslog")?;
        socket.set_write_timeout(Some(std::time::Duration::from_millis(100)))?;
        Some(socket)
    } else {
        None
    };
    *LOGGER
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Logger {
        file,
        syslog,
        colors: options.colors,
    };
    Ok(())
}

pub fn write(arguments: fmt::Arguments<'_>) {
    let message = arguments.to_string();
    let mut logger = LOGGER
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut stderr = std::io::stderr().lock();
    if logger.colors && std::io::stderr().is_terminal() {
        let _ = writeln!(stderr, "\x1b[36m{message}\x1b[0m");
    } else {
        let _ = writeln!(stderr, "{message}");
    }
    if let Some(file) = &mut logger.file {
        let _ = writeln!(file, "{message}");
    }
    if let Some(socket) = &logger.syslog {
        let _ = socket.send(format!("<14>rxminer[{}]: {message}", std::process::id()).as_bytes());
    }
}

#[macro_export]
macro_rules! status { ($($arg:tt)*) => { $crate::logging::write(format_args!($($arg)*)) }; }
