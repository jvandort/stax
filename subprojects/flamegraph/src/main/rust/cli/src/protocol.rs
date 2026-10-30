// The client <-> daemon protocol, over a Unix socket, one connection per command.
//
// Request: `stax/<version>`, then each command-line argument after `stax`, each terminated by a
// NUL byte (which cannot occur in a Unix argument, so nothing needs escaping). The client then
// shuts down its write side.
//
// Response: one status byte (0: success, printed to stdout; 1: error, printed to stderr), then
// UTF-8 text until the daemon closes the connection.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

pub const VERSION_TAG: &str = concat!("stax/", env!("CARGO_PKG_VERSION"));

pub const SUCCESS: u8 = 0;
pub const FAILURE: u8 = 1;

/// `~/.stax`: holds the daemon log, and the socket when `$XDG_RUNTIME_DIR` is unset.
pub fn state_dir() -> io::Result<PathBuf> {
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => Ok(PathBuf::from(home).join(".stax")),
        _ => Err(io::Error::new(io::ErrorKind::NotFound, "HOME is not set")),
    }
}

/// The directory holding the socket: `$XDG_RUNTIME_DIR/stax` (the per-user runtime directory on
/// Linux), else `~/.stax`.
pub fn socket_dir() -> io::Result<PathBuf> {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir).join("stax")),
        _ => state_dir(),
    }
}

pub fn socket_path() -> io::Result<PathBuf> {
    Ok(socket_dir()?.join("default.sock"))
}

pub fn log_path() -> io::Result<PathBuf> {
    Ok(state_dir()?.join("daemon.log"))
}

/// Creates `dir` and its parents, readable only by the current user.
pub fn create_private_dir(dir: &std::path::Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

pub fn write_request(stream: &mut UnixStream, args: &[OsString]) -> io::Result<()> {
    let mut buf = Vec::new();
    buf.extend_from_slice(VERSION_TAG.as_bytes());
    buf.push(0);
    for arg in args {
        buf.extend_from_slice(arg.as_bytes());
        buf.push(0);
    }
    stream.write_all(&buf)?;
    stream.shutdown(std::net::Shutdown::Write)
}

/// Reads a request, returning its version tag and arguments.
pub fn read_request(stream: &mut UnixStream) -> io::Result<(String, Vec<OsString>)> {
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?;
    if buf.is_empty() {
        // `stax serve` probes the socket by connecting without sending anything.
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "empty request"));
    }
    let Some(body) = buf.strip_suffix(&[0]) else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "request is not NUL-terminated"));
    };
    let mut fields = body.split(|&b| b == 0);
    let version = String::from_utf8_lossy(fields.next().unwrap_or_default()).into_owned();
    let args = fields.map(|f| OsStr::from_bytes(f).to_os_string()).collect();
    Ok((version, args))
}

pub fn write_response(stream: &mut UnixStream, status: u8, text: &str) -> io::Result<()> {
    let mut buf = Vec::with_capacity(text.len() + 1);
    buf.push(status);
    buf.extend_from_slice(text.as_bytes());
    stream.write_all(&buf)
}

/// Reads a response, returning its status byte and text.
pub fn read_response(stream: &mut UnixStream) -> io::Result<(u8, String)> {
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?;
    let Some((&status, text)) = buf.split_first() else {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the daemon closed the connection without replying"));
    };
    Ok((status, String::from_utf8_lossy(text).into_owned()))
}
