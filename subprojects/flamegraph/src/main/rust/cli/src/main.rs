mod cli;
mod commands;
mod daemon;
mod graph;
mod protocol;
mod summary;
mod table;

use std::ffi::OsString;
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> ExitCode {
    // The guide, for `stax`, `stax -h`, `stax --help` and `stax help`.
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.is_empty() || (args.len() == 1 && ["-h", "--help", "help"].iter().any(|h| args[0] == *h)) {
        print!("{}", cli::GUIDE);
        return ExitCode::SUCCESS;
    }
    // Parse locally first, so `--help` and usage errors work without a daemon.
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Daemon => daemon::run(),
        Command::Load { paths, raw } => {
            // The daemon has a different working directory, so send absolute paths.
            let mut args = vec![OsString::from("load")];
            if raw {
                args.push(OsString::from("--raw"));
            }
            for path in paths {
                match std::fs::canonicalize(&path) {
                    Ok(path) => args.push(path.into_os_string()),
                    Err(e) => {
                        eprintln!("{}: {e}", path.display());
                        return ExitCode::FAILURE;
                    }
                }
            }
            ensure_daemon().and_then(|started| if started { send(&args) } else { Ok(ExitCode::FAILURE) })
        }
        _ => send(&args),
    };
    result.unwrap_or_else(|e| {
        // The reader of our output went away (e.g. `stax ... | head`): nothing left to report.
        if e.kind() == io::ErrorKind::BrokenPipe {
            return ExitCode::SUCCESS;
        }
        eprintln!("stax: {e}");
        ExitCode::FAILURE
    })
}

/// Starts the daemon unless one is running. False if it failed to start.
fn ensure_daemon() -> io::Result<bool> {
    if UnixStream::connect(protocol::socket_path()?).is_ok() {
        return Ok(true);
    }
    daemon::start()
}

/// Sends a command to the daemon and prints its response.
fn send(args: &[OsString]) -> io::Result<ExitCode> {
    let socket = protocol::socket_path()?;
    let mut stream = match UnixStream::connect(&socket) {
        Ok(stream) => stream,
        Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => {
            eprintln!("No stax daemon is running, so nothing is loaded. Load a profile with `stax load <file>...`.");
            return Ok(ExitCode::FAILURE);
        }
        Err(e) => return Err(e),
    };
    protocol::write_request(&mut stream, args)?;
    let (status, mut text) = protocol::read_response(&mut stream)?;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if status == protocol::SUCCESS {
        io::stdout().write_all(text.as_bytes())?;
        Ok(ExitCode::SUCCESS)
    } else {
        io::stderr().write_all(text.as_bytes())?;
        Ok(ExitCode::from(status))
    }
}
