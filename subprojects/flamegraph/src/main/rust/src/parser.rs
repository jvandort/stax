use wasm_bindgen::prelude::*;

use crate::{Error, GraphBuilder, WasmStackGraph};

/// Adds line `line_number` (1-based, for error messages) to the graph.
fn parse_line(line: &[u8], line_number: u64, graph: &mut GraphBuilder) -> Result<(), Error> {
    let malformed = || Error::MalformedLine(line_number, String::from_utf8_lossy(line).into_owned());
    let last_space = memchr::memrchr(b' ', line).ok_or_else(malformed)?;

    let count = &line[last_space + 1..];
    if count.is_empty() || !count.iter().all(u8::is_ascii_digit) {
        return Err(malformed());
    }
    let mut value: i64 = 0;
    for &b in count {
        value = value.checked_mul(10).and_then(|v| v.checked_add((b - b'0') as i64)).ok_or_else(malformed)?;
    }

    let frames = std::str::from_utf8(&line[..last_space]).map_err(|_| malformed())?;
    let mut parent = 0usize;
    let mut prev = 0usize;

    for pos in memchr::memchr_iter(b';', frames.as_bytes()) {
        if pos > prev {
            (parent, _) = graph.get_or_create_child(parent, &frames[prev..pos], value);
        }
        prev = pos + 1;
    }
    // Last (or only) frame
    if prev < frames.len() {
        graph.get_or_create_child(parent, &frames[prev..], value);
    }

    graph.values[0] += value;
    Ok(())
}

/// Streaming stack parser — processes decompressed chunks as they arrive.
///
/// JS workflow:
///   ```js
///   const parser = new StacksParser()
///   // for each decompressed chunk:
///   parser.feed(chunk)
///   // when done:
///   const graph = parser.finish()
///   ```
///
/// Partial lines that span chunk boundaries are buffered internally, so chunks of
/// any size (including single-byte chunks) are handled correctly.
#[wasm_bindgen]
pub struct StacksParser {
    graph: GraphBuilder,
    /// Bytes of an incomplete line carried over from the previous chunk.
    line_buf: Vec<u8>,
    /// Lines completed so far, for error messages.
    lines: u64,
}

#[wasm_bindgen]
impl StacksParser {
    #[wasm_bindgen(constructor)]
    pub fn new(root_name: &str) -> Self {
        StacksParser {
            graph: GraphBuilder::new(root_name),
            line_buf: Vec::new(),
            lines: 0,
        }
    }

    /// Feed a decompressed chunk into the parser.
    /// May be called any number of times. Chunks may be any size.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), Error> {
        let mut start = 0;
        for pos in memchr::memchr_iter(b'\n', chunk) {
            self.lines += 1;
            if self.line_buf.is_empty() {
                // No pending partial data — process the slice directly.
                let line_end = if pos > start && chunk[pos - 1] == b'\r' { pos - 1 } else { pos };
                if line_end > start {
                    parse_line(&chunk[start..line_end], self.lines, &mut self.graph)?;
                }
            } else {
                // Complete the pending partial line.
                self.line_buf.extend_from_slice(&chunk[start..pos]);
                // Strip trailing \r (handles \r\n split across chunks).
                if self.line_buf.last() == Some(&b'\r') {
                    self.line_buf.pop();
                }
                if !self.line_buf.is_empty() {
                    parse_line(&self.line_buf, self.lines, &mut self.graph)?;
                }
                self.line_buf.clear();
            }
            start = pos + 1;
        }
        // Buffer any remaining bytes as a partial line.
        if start < chunk.len() {
            self.line_buf.extend_from_slice(&chunk[start..]);
        }
        Ok(())
    }

    /// Finish parsing and return the completed graph.
    /// Consumes the parser. Do not call `feed` after this.
    pub fn finish(mut self) -> Result<WasmStackGraph, Error> {
        if !self.line_buf.is_empty() {
            // Strip trailing \r (in case the last line ended with \r without \n).
            if self.line_buf.last() == Some(&b'\r') {
                self.line_buf.pop();
            }
            parse_line(&self.line_buf, self.lines + 1, &mut self.graph)?;
        }
        Ok(self.graph.into_wasm_stack_graph())
    }
}

/// Test helper: parse stack collapse data in one shot via StacksParser.
#[cfg(test)]
pub fn parse_stacks_impl(data: &[u8]) -> WasmStackGraph {
    let mut parser = StacksParser::new("root");
    parser.feed(data).expect("parse error in feed");
    let graph = parser.finish().expect("parse error in finish");
    crate::testing::assert_valid_graph(&graph);
    graph
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(data: &[u8]) -> Result<WasmStackGraph, Error> {
        let mut parser = StacksParser::new("root");
        parser.feed(data)?;
        parser.finish()
    }

    #[test]
    fn test_parses_counts() {
        let g = parse(b"a;b 3\r\na 2\n").unwrap();
        assert_eq!(g.values[0], 5);
    }

    #[test]
    fn test_non_numeric_count_is_an_error() {
        assert!(matches!(parse(b"a;b x3\n"), Err(Error::MalformedLine(..))));
        assert!(matches!(parse(b"a;b \n"), Err(Error::MalformedLine(..))));
        assert!(matches!(parse(b"plugins {\n"), Err(Error::MalformedLine(..))));
    }

    #[test]
    fn test_overflowing_count_is_an_error() {
        assert!(matches!(parse(b"a 99999999999999999999\n"), Err(Error::MalformedLine(..))));
    }

    #[test]
    fn test_invalid_utf8_is_an_error() {
        assert!(matches!(parse(b"a;\xff\xfe 1\n"), Err(Error::MalformedLine(..))));
    }

    #[test]
    fn test_errors_give_the_line_number() {
        let line_of = |chunks: &[&[u8]]| {
            let mut parser = StacksParser::new("root");
            let result = chunks.iter().try_for_each(|c| parser.feed(c)).and_then(|()| parser.finish().map(|_| ()));
            match result {
                Err(Error::MalformedLine(n, _)) => n,
                other => panic!("expected a malformed line, got {other:?}"),
            }
        };
        assert_eq!(line_of(&[b"a 1\n\nb x\n"]), 3);
        assert_eq!(line_of(&[b"a 1\nb", b" 2\nc x"]), 3);
        assert_eq!(line_of(&[b"a 1\r\nb 2\n", b"c", b" x\n"]), 3);
    }
}
