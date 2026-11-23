use std::fmt;

use wasm_bindgen::prelude::*;

/// Errors raised by graph parsing and transformation.
///
/// Exported `#[wasm_bindgen]` functions return `Result<T, Error>` directly: wasm-bindgen converts
/// the error into a JS `Error` (via the `From<Error> for JsValue` impl below) only when the result
/// crosses into JS, so the same functions are callable natively (e.g. from the CLI) where
/// constructing a JS value would panic.
#[derive(Debug)]
pub enum Error {
    /// A `-stacks.txt` line that isn't UTF-8 frames, a space, and a sample count: its 1-based
    /// number and its text.
    MalformedLine(u64, String),
    /// The input does not start with the JFR chunk magic.
    NotJfr,
    /// A JFR chunk header declares a size smaller than the header itself.
    InvalidJfrChunkSize(u64),
    /// A JFR chunk header declares a size that does not fit in memory.
    JfrChunkTooLarge(u64),
    /// The JFR reader failed to decode a chunk.
    JfrChunk(jfrs::reader::Error),
    /// The JFR reader failed to decode an event.
    JfrEvent(jfrs::reader::Error),
    /// The JFR input ended part-way through a chunk; holds the number of leftover bytes.
    TruncatedJfr(usize),
    /// The JFR recording contains no supported sample events.
    NoJfrSamples,
    /// No node in the graph has the given name.
    NoSuchName(String),
    /// The root node cannot be deleted.
    DeleteRoot,
    /// A non-root node has no parent.
    NoParent(usize),
    /// A decoded JFR graph index was out of range or already taken.
    GraphTaken,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::MalformedLine(number, line) => {
                write!(f, "line {number}: malformed input: expected `frame;frame;... count`, got: {line}")
            }
            Error::NotJfr => write!(f, "Not a JFR file (bad chunk magic)"),
            Error::InvalidJfrChunkSize(size) => write!(f, "Invalid JFR chunk size: {size}"),
            Error::JfrChunkTooLarge(size) => write!(f, "JFR chunk too large: {size} bytes"),
            Error::JfrChunk(e) => write!(f, "Failed to read JFR chunk: {e}"),
            Error::JfrEvent(e) => write!(f, "Failed to read JFR event: {e}"),
            Error::TruncatedJfr(remaining) => {
                write!(f, "Truncated JFR input: {remaining} bytes of an incomplete chunk remain")
            }
            Error::NoJfrSamples => write!(
                f,
                "No sample events found in this JFR (looked for jdk.ExecutionSample, \
                 jdk.NativeMethodSample, and profiler.WallClockSample)"
            ),
            Error::NoSuchName(name) => write!(f, "No nodes with name: {name}"),
            Error::DeleteRoot => write!(f, "cannot delete the root node"),
            Error::NoParent(node) => write!(f, "node {node} has no parent in a non-empty graph"),
            Error::GraphTaken => write!(f, "Graph index out of range or already taken"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::JfrChunk(e) | Error::JfrEvent(e) => Some(e),
            _ => None,
        }
    }
}

impl From<Error> for JsValue {
    fn from(e: Error) -> Self {
        JsError::new(&e.to_string()).into()
    }
}
