use std::io::Cursor;

use ahash::AHashMap;
use jfrs::reader::event::Accessor;
use jfrs::reader::value_descriptor::ValueDescriptor;
use jfrs::reader::JfrReader;
use wasm_bindgen::prelude::*;

use crate::{GraphBuilder, WasmStackGraph};

/// Decodes one JVM type from the front of a method-descriptor string,
/// returning its Java source form and the remaining input:
/// `Ljava/lang/String;I)V` → (`java.lang.String`, `I)V`).
fn decode_java_type(s: &str) -> Option<(String, &str)> {
    let bytes = s.as_bytes();
    let mut dims = 0;
    while bytes.get(dims) == Some(&b'[') {
        dims += 1;
    }
    let (base, rest) = match *bytes.get(dims)? {
        b'B' => ("byte", &s[dims + 1..]),
        b'C' => ("char", &s[dims + 1..]),
        b'D' => ("double", &s[dims + 1..]),
        b'F' => ("float", &s[dims + 1..]),
        b'I' => ("int", &s[dims + 1..]),
        b'J' => ("long", &s[dims + 1..]),
        b'S' => ("short", &s[dims + 1..]),
        b'Z' => ("boolean", &s[dims + 1..]),
        b'L' => {
            let semi = s[dims..].find(';')? + dims;
            (&s[dims + 1..semi], &s[semi + 1..])
        }
        _ => return None,
    };
    let mut ty = base.replace('/', ".");
    for _ in 0..dims {
        ty.push_str("[]");
    }
    Some((ty, rest))
}

/// Decodes a JVM method descriptor's parameter list into the Java source form
/// used by JFR-converted stacks: `(Ljava/lang/String;I[J)V` →
/// `java.lang.String, int, long[]`.
fn decode_descriptor_params(descriptor: &str) -> Option<String> {
    let inner = descriptor.strip_prefix('(')?;
    let mut rest = &inner[..inner.find(')')?];
    let mut out = String::new();
    while !rest.is_empty() {
        let (ty, remaining) = decode_java_type(rest)?;
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(&ty);
        rest = remaining;
    }
    Some(out)
}

/// Maps a JFR frame-type description to the async-profiler-style suffix used
/// throughout the tool. Frame types that don't describe a JVM compilation
/// state (native methods, C++ frames) get no suffix.
fn frame_type_suffix(description: Option<&str>) -> &'static str {
    match description {
        Some("Interpreted") => "_[0]",
        Some("JIT compiled") => "_[j]",
        Some("Inlined") => "_[i]",
        Some("C1 compiled") => "_[1]",
        Some("Kernel") => "_[k]",
        _ => "",
    }
}

fn accessor_str<'a>(acc: Option<Accessor<'a>>) -> Option<&'a str> {
    acc.and_then(|a| <&str>::try_from(a.value).ok())
}

const ACC_BRIDGE: i32 = 0x40;

/// Whether the frame's method is a compiler-generated bridge (ACC_BRIDGE):
/// a trivial synthetic delegator with the same name and parameter types as
/// the method it forwards to, differing only in return type. Since frame
/// names drop return types and line numbers, a bridge renders identically to
/// its target, producing spurious same-name parent/child frame pairs.
fn is_bridge_frame(frame: &Accessor) -> bool {
    frame
        .get_field("method")
        .and_then(|m| m.get_field("modifiers"))
        .and_then(|v| i32::try_from(v.value).ok())
        .is_some_and(|m| m & ACC_BRIDGE != 0)
}

/// Formats a JFR stack frame as `pkg.Class.method(params)_[type]`, matching
/// the naming produced by gradle-profiler's JFR conversion (dotted class
/// names, Java-source parameter types, no line numbers). Returns `None` for
/// bridge frames, which are dropped: whether a bridge shows up next to its
/// same-named target varies with JIT state, so keeping them makes the same
/// call path decode to different stacks in different recordings.
fn frame_name(frame: &Accessor) -> Option<String> {
    if is_bridge_frame(frame) {
        return None;
    }
    let method = frame.get_field("method");
    let class_name = accessor_str(
        method
            .as_ref()
            .and_then(|m| m.get_field("type"))
            .and_then(|t| t.get_field("name"))
            .and_then(|n| n.get_field("string")),
    )
    .unwrap_or("");
    let method_name = accessor_str(
        method
            .as_ref()
            .and_then(|m| m.get_field("name"))
            .and_then(|n| n.get_field("string")),
    )
    .unwrap_or("");
    let descriptor = accessor_str(
        method
            .as_ref()
            .and_then(|m| m.get_field("descriptor"))
            .and_then(|d| d.get_field("string")),
    );
    let type_description =
        accessor_str(frame.get_field("type").and_then(|t| t.get_field("description")));

    if class_name.is_empty() && method_name.is_empty() {
        return Some("[unknown]".to_string());
    }

    let mut name = String::new();
    if !class_name.is_empty() {
        // JFR symbols hold JVM-internal names (java/lang/Thread).
        for c in class_name.chars() {
            name.push(if c == '/' { '.' } else { c });
        }
        name.push('.');
    }
    name.push_str(if method_name.is_empty() { "[unknown]" } else { method_name });
    if let Some(descriptor) = descriptor {
        name.push('(');
        name.push_str(&decode_descriptor_params(descriptor).unwrap_or_else(|| "...".to_string()));
        name.push(')');
    }
    name.push_str(frame_type_suffix(type_description));
    Some(name)
}

/// Returns the constant-pool reference of a frame's field without resolving
/// it, for use as a cache key. `None` when the value is stored inline.
fn cp_index(acc: Option<Accessor>) -> Option<(i64, i64)> {
    match acc?.value {
        ValueDescriptor::ConstantPool { class_id, constant_index } => {
            Some((*class_id, *constant_index))
        }
        _ => None,
    }
}

/// The sample weight of one event. Execution/native-method samples each count
/// as one sample; async-profiler 4.0 wall-clock events batch consecutive
/// identical samples into a `samples` field.
fn event_weight(event_class: &str, value: &Accessor) -> i64 {
    if event_class != "profiler.WallClockSample" {
        return 1;
    }
    let samples = value.get_field("samples");
    samples
        .as_ref()
        .and_then(|s| i64::try_from(s.value).ok())
        .or_else(|| samples.as_ref().and_then(|s| i32::try_from(s.value).ok().map(i64::from)))
        .map_or(1, |s| s.max(1))
}

/// Decodes every JFR chunk in `data` (which must contain only whole chunks)
/// and folds its sample events into the cpu/wall builders.
fn fold_chunks(
    data: &[u8],
    cpu: &mut GraphBuilder,
    wall: &mut GraphBuilder,
    cpu_used: &mut bool,
    wall_used: &mut bool,
) -> Result<(), String> {
    let mut reader = JfrReader::new(Cursor::new(data));
    for chunk in reader.chunks() {
        let (mut chunk_reader, chunk) =
            chunk.map_err(|e| format!("Failed to read JFR chunk: {e}"))?;
        // Frame names repeat heavily via the constant pool; cache the formatted
        // name per (method ref, frame-type ref). Pool indices are per-chunk.
        // `None` marks a bridge frame, which is dropped from the stack.
        let mut name_cache: AHashMap<((i64, i64), (i64, i64)), Option<String>> = AHashMap::new();

        for event in chunk_reader.events(&chunk) {
            let event = event.map_err(|e| format!("Failed to read JFR event: {e}"))?;
            let (builder, used) = match event.class.name() {
                "jdk.ExecutionSample" | "jdk.NativeMethodSample" => (&mut *cpu, &mut *cpu_used),
                "profiler.WallClockSample" => (&mut *wall, &mut *wall_used),
                _ => continue,
            };

            let value = event.value();
            let weight = event_weight(event.class.name(), &value);
            let Some(frames) = value
                .get_field("stackTrace")
                .and_then(|st| st.get_field("frames"))
                .and_then(|f| f.as_iter())
            else {
                continue;
            };

            // JFR stacks are leaf-first; the graph is built root-first.
            let frames: Vec<Accessor> = frames.collect();
            if frames.is_empty() {
                continue;
            }
            let mut parent = 0usize;
            for frame in frames.iter().rev() {
                let key = cp_index(frame.get_field_raw("method"))
                    .map(|m| (m, cp_index(frame.get_field_raw("type")).unwrap_or((-1, -1))));
                let uncached;
                let name: &str = match key {
                    Some(key) => {
                        match name_cache.entry(key).or_insert_with(|| frame_name(frame)) {
                            Some(name) => name,
                            None => continue,
                        }
                    }
                    None => match frame_name(frame) {
                        Some(name) => {
                            uncached = name;
                            &uncached
                        }
                        None => continue,
                    },
                };
                (parent, _) = builder.get_or_create_child(parent, name, weight);
            }
            builder.values[0] += weight;
            *used = true;
        }
    }
    Ok(())
}

const JFR_MAGIC: [u8; 4] = [b'F', b'L', b'R', 0];
/// Bytes needed to frame a chunk: magic (4) + version (4) + chunk size (8).
const CHUNK_PREFIX_LEN: usize = 16;
/// The fixed chunk header is 68 bytes; no valid chunk can be smaller.
const MIN_CHUNK_SIZE: u64 = 68;

/// Streaming JFR parser — feed byte chunks as they arrive.
///
/// A JFR event cannot be decoded until its chunk's trailing metadata and
/// constant pools are available, so bytes are buffered until one whole JFR
/// chunk is present, then that chunk is decoded and folded into the graphs
/// and its bytes are discarded. JFR chunks are self-contained, so peak memory
/// is one JFR chunk (the JVM caps them at ~12MB by default) regardless of
/// recording size.
///
/// JS workflow mirrors [`crate::StacksParser`]: `feed(chunk)` any number of
/// times, then `finish()`.
#[wasm_bindgen]
pub struct JfrParser {
    /// Bytes of the incomplete trailing JFR chunk.
    pending: Vec<u8>,
    cpu: GraphBuilder,
    wall: GraphBuilder,
    cpu_used: bool,
    wall_used: bool,
}

impl Default for JfrParser {
    fn default() -> Self {
        Self::new()
    }
}

impl JfrParser {
    fn feed_impl(&mut self, chunk: &[u8]) -> Result<(), String> {
        self.pending.extend_from_slice(chunk);
        let mut start = 0usize;
        loop {
            let rest = &self.pending[start..];
            if rest.len() < CHUNK_PREFIX_LEN {
                break;
            }
            if rest[..4] != JFR_MAGIC {
                return Err("Not a JFR file (bad chunk magic)".to_string());
            }
            let chunk_size = u64::from_be_bytes(rest[8..16].try_into().unwrap());
            if chunk_size < MIN_CHUNK_SIZE {
                return Err(format!("Invalid JFR chunk size: {chunk_size}"));
            }
            let Ok(chunk_size) = usize::try_from(chunk_size) else {
                return Err(format!("JFR chunk too large: {chunk_size} bytes"));
            };
            if rest.len() < chunk_size {
                break;
            }
            fold_chunks(
                &self.pending[start..start + chunk_size],
                &mut self.cpu,
                &mut self.wall,
                &mut self.cpu_used,
                &mut self.wall_used,
            )?;
            start += chunk_size;
        }
        if start > 0 {
            self.pending.drain(..start);
        }
        Ok(())
    }

    fn finish_impl(self) -> Result<Vec<(String, WasmStackGraph)>, String> {
        if !self.pending.is_empty() {
            return Err(format!(
                "Truncated JFR input: {} bytes of an incomplete chunk remain",
                self.pending.len()
            ));
        }
        let mut graphs = Vec::new();
        if self.cpu_used {
            graphs.push(("cpu".to_string(), self.cpu.into_wasm_stack_graph()));
        }
        if self.wall_used {
            graphs.push(("wall".to_string(), self.wall.into_wasm_stack_graph()));
        }
        if graphs.is_empty() {
            return Err("No sample events found in this JFR (looked for jdk.ExecutionSample, \
                        jdk.NativeMethodSample, and profiler.WallClockSample)"
                .to_string());
        }
        Ok(graphs)
    }
}

#[wasm_bindgen]
impl JfrParser {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        JfrParser {
            pending: Vec::new(),
            cpu: GraphBuilder::new("root"),
            wall: GraphBuilder::new("root"),
            cpu_used: false,
            wall_used: false,
        }
    }

    /// Feed the next bytes of the recording. Chunks may be any size.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), JsError> {
        self.feed_impl(chunk).map_err(|e| JsError::new(&e))
    }

    /// Finish parsing and return the decoded graphs, one per sample kind
    /// present. Consumes the parser. Errors on truncated input or when the
    /// recording contains no supported sample events.
    pub fn finish(self) -> Result<WasmJfrGraphs, JsError> {
        let graphs = self.finish_impl().map_err(|e| JsError::new(&e))?;
        let (kinds, graphs) = graphs.into_iter().map(|(k, g)| (k, Some(g))).unzip();
        Ok(WasmJfrGraphs { kinds, graphs })
    }
}

/// Test helper: decode a complete JFR recording in one shot via [`JfrParser`].
#[cfg(test)]
pub(crate) fn parse_jfr_impl(data: &[u8]) -> Result<Vec<(String, WasmStackGraph)>, String> {
    let mut parser = JfrParser::new();
    parser.feed_impl(data)?;
    parser.finish_impl()
}

/// The graphs decoded from one JFR file, one per sample kind present:
/// `cpu` (jdk.ExecutionSample + jdk.NativeMethodSample) and `wall`
/// (async-profiler 4.0's profiler.WallClockSample). Values are sample counts.
#[wasm_bindgen]
pub struct WasmJfrGraphs {
    kinds: Vec<String>,
    graphs: Vec<Option<WasmStackGraph>>,
}

#[wasm_bindgen]
impl WasmJfrGraphs {
    pub fn count(&self) -> u32 {
        self.kinds.len() as u32
    }

    pub fn kind(&self, i: u32) -> String {
        self.kinds[i as usize].clone()
    }

    /// Transfers ownership of graph `i` to the caller. Each graph can be
    /// taken exactly once.
    pub fn take_graph(&mut self, i: u32) -> Result<WasmStackGraph, JsError> {
        self.graphs
            .get_mut(i as usize)
            .and_then(Option::take)
            .ok_or_else(|| JsError::new("Graph index out of range or already taken"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::get_name;

    #[test]
    fn test_decode_descriptor_params() {
        let cases: &[(&str, Option<&str>)] = &[
            ("()V", Some("")),
            ("(I)V", Some("int")),
            ("(Ljava/lang/String;I[J)V", Some("java.lang.String, int, long[]")),
            ("([[Ljava/lang/Object;)Ljava/lang/Object;", Some("java.lang.Object[][]")),
            ("(BCDFIJSZ)V", Some("byte, char, double, float, int, long, short, boolean")),
            ("(Q)V", None),
            ("no parens", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                decode_descriptor_params(input).as_deref(),
                *expected,
                "input: {input}"
            );
        }
    }

    #[test]
    fn test_frame_type_suffixes() {
        assert_eq!(frame_type_suffix(Some("Interpreted")), "_[0]");
        assert_eq!(frame_type_suffix(Some("JIT compiled")), "_[j]");
        assert_eq!(frame_type_suffix(Some("Inlined")), "_[i]");
        assert_eq!(frame_type_suffix(Some("C1 compiled")), "_[1]");
        assert_eq!(frame_type_suffix(Some("Kernel")), "_[k]");
        assert_eq!(frame_type_suffix(Some("Native")), "");
        assert_eq!(frame_type_suffix(None), "");
    }

    /// A ~2s recording of a busy loop with only jdk.ExecutionSample enabled:
    /// Busy.main -> Busy.get() (via a Supplier<Long> bridge method) ->
    /// Busy.fib recursion. See test-data/README.md.
    const CPU_SAMPLE_JFR: &[u8] =
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/cpu-sample.jfr"));

    fn node_names(g: &WasmStackGraph) -> Vec<&str> {
        (0..g.node_count() as usize)
            .map(|i| get_name(&g.names_data, &g.names_offsets, i))
            .collect()
    }

    /// Whether any node whose name starts with `prefix` has a direct child
    /// whose name also starts with `prefix`.
    fn has_same_prefix_child(g: &WasmStackGraph, prefix: &str) -> bool {
        let names = node_names(g);
        (0..names.len()).any(|i| {
            names[i].starts_with(prefix)
                && crate::get_children(&g.children_offsets, &g.children_data, i)
                    .iter()
                    .any(|&c| names[c as usize].starts_with(prefix))
        })
    }

    #[test]
    fn test_decode_cpu_samples() {
        let graphs = parse_jfr_impl(CPU_SAMPLE_JFR).expect("fixture should decode");
        assert_eq!(graphs.len(), 1);
        let (kind, graph) = &graphs[0];
        assert_eq!(kind, "cpu");
        assert!(graph.values[0] > 0, "root should have samples");

        let names = node_names(graph);
        let fib = names.iter().find(|n| n.starts_with("Busy.fib(int)"));
        assert!(fib.is_some(), "expected a Busy.fib(int) frame in {names:?}");
        // Class names are dotted and parameters decoded from the descriptor.
        let main = names.iter().find(|n| n.starts_with("Busy.main(java.lang.String[])"));
        assert!(main.is_some(), "expected a Busy.main frame in {names:?}");
    }

    #[test]
    fn test_no_samples_is_an_error() {
        assert!(parse_jfr_impl(b"not a jfr").is_err());
    }

    #[test]
    fn test_bad_magic_is_an_error() {
        assert!(parse_jfr_impl(b"this is definitely not a jfr file").is_err());
    }

    #[test]
    fn test_chunked_feed_matches_one_shot() {
        let one_shot = parse_jfr_impl(CPU_SAMPLE_JFR).unwrap();
        let mut parser = JfrParser::new();
        for chunk in CPU_SAMPLE_JFR.chunks(4096) {
            parser.feed_impl(chunk).unwrap();
        }
        let streamed = parser.finish_impl().unwrap();
        assert_eq!(streamed.len(), one_shot.len());
        assert_eq!(streamed[0].0, one_shot[0].0);
        assert_eq!(streamed[0].1.values[0], one_shot[0].1.values[0]);
        assert_eq!(streamed[0].1.node_count(), one_shot[0].1.node_count());
    }

    #[test]
    fn test_multiple_jfr_chunks_fold_into_one_graph() {
        // A JFR file is a plain sequence of self-contained chunks, so two
        // concatenated recordings form a valid multi-chunk stream. This also
        // exercises the buffer drain between chunks.
        let mut doubled = CPU_SAMPLE_JFR.to_vec();
        doubled.extend_from_slice(CPU_SAMPLE_JFR);
        let single = parse_jfr_impl(CPU_SAMPLE_JFR).unwrap();
        let double = parse_jfr_impl(&doubled).unwrap();
        assert_eq!(double[0].1.values[0], 2 * single[0].1.values[0]);
    }

    #[test]
    fn test_truncated_jfr_is_an_error() {
        let truncated = &CPU_SAMPLE_JFR[..CPU_SAMPLE_JFR.len() - 100];
        let Err(err) = parse_jfr_impl(truncated) else {
            panic!("truncated input should fail");
        };
        assert!(err.contains("Truncated"), "unexpected error: {err}");
    }

    /// Busy implements Supplier<Long>, so raw stacks contain the synthetic
    /// bridge `Object get()` directly above the real `Long get()` — rendered
    /// identically once return types are dropped. The bridge frame must be
    /// skipped, otherwise whether the pair collapses varies with JIT state
    /// and the same call path decodes to different stacks across recordings.
    #[test]
    fn test_bridge_frames_are_skipped() {
        let graphs = parse_jfr_impl(CPU_SAMPLE_JFR).unwrap();
        let (_, graph) = &graphs[0];
        let names = node_names(graph);
        assert!(
            names.iter().any(|n| n.starts_with("Busy.get()")),
            "the real Busy.get() frame should survive"
        );
        assert!(
            !has_same_prefix_child(graph, "Busy.get()"),
            "bridge frame should not stack Busy.get() on Busy.get()"
        );
    }

    /// Genuine recursion is not a bridge and must be preserved.
    #[test]
    fn test_recursive_frames_are_preserved() {
        let graphs = parse_jfr_impl(CPU_SAMPLE_JFR).unwrap();
        let (_, graph) = &graphs[0];
        assert!(
            has_same_prefix_child(graph, "Busy.fib(int)"),
            "recursive Busy.fib frames should stack on each other"
        );
    }

    /// Validation against real recordings: set `JFR_TEST_FILE` to a .jfr path
    /// and run `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore = "requires JFR_TEST_FILE env var pointing at a real .jfr"]
    fn test_decode_external_jfr() {
        let path = std::env::var("JFR_TEST_FILE").expect("set JFR_TEST_FILE to a .jfr path");
        let data = std::fs::read(&path).expect("failed to read JFR_TEST_FILE");
        let graphs = parse_jfr_impl(&data).expect("failed to decode");
        for (kind, graph) in &graphs {
            let names = node_names(graph);
            let mut top: Vec<(i64, &str)> = graph
                .values
                .iter()
                .copied()
                .zip(names.iter().copied())
                .skip(1)
                .collect();
            top.sort_unstable_by_key(|(v, _)| -v);
            println!(
                "{path}: kind={kind} nodes={} root_samples={}",
                graph.node_count(),
                graph.values[0]
            );
            for (value, name) in top.iter().take(10) {
                println!("  {value:>8}  {name}");
            }
        }
    }
}
