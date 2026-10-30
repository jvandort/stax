mod cli;
mod commands;
mod daemon;
mod graph;
mod protocol;
mod summary;

use std::ffi::OsString;
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> ExitCode {
    // Parse locally first, so `--help` and usage errors work without a daemon.
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Serve => daemon::serve(),
        Command::Daemon => daemon::run(),
        Command::Load { paths } => {
            // The daemon has a different working directory, so send absolute paths.
            let mut args = vec![OsString::from("load")];
            for path in paths {
                match std::fs::canonicalize(&path) {
                    Ok(path) => args.push(path.into_os_string()),
                    Err(e) => {
                        eprintln!("{}: {e}", path.display());
                        return ExitCode::FAILURE;
                    }
                }
            }
            send(&args)
        }
        _ => send(&std::env::args_os().skip(1).collect::<Vec<_>>()),
    };
    result.unwrap_or_else(|e| {
        eprintln!("stax: {e}");
        ExitCode::FAILURE
    })
}

/// Sends a command to the daemon and prints its response.
fn send(args: &[OsString]) -> io::Result<ExitCode> {
    let socket = protocol::socket_path()?;
    let mut stream = match UnixStream::connect(&socket) {
        Ok(stream) => stream,
        Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => {
            eprintln!("no stax daemon running; start one with `stax serve`");
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
