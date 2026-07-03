use std::borrow::Cow;

use wasm_bindgen::prelude::*;

use crate::{get_children, get_name, GraphBuilder, WasmStackGraph};

/// Normalizes frame-type suffixes that only reflect JVM execution state:
/// `_[0]` (interpreted), `_[1]` (C1-compiled), and `_[i]` (inlined) become
/// `_[j]` (JIT-compiled). Which tier a method executes in — and whether the
/// JIT inlined it — varies with warmup, not code, so the same method merges
/// to one node regardless of compilation state.
fn normalize_frame_type(name: &str) -> Option<String> {
    const EQUIVALENT_SUFFIXES: &[&str] = &["_[0]", "_[1]", "_[i]"];
    EQUIVALENT_SUFFIXES
        .iter()
        .find_map(|s| name.strip_suffix(s))
        .map(|base| format!("{base}_[j]"))
}

const ID_SEPARATORS: [char; 3] = ['.', '/', '+'];

fn is_id_separator(b: Option<&u8>) -> bool {
    matches!(b, Some(b'.') | Some(b'/') | Some(b'+'))
}

/// Consumes the run of separator-prefixed generated-id segments — a hex
/// address (`+0x…`, `.0x…`, `/0x…`) or a plain number (`.123`) — starting at
/// `start`. Each segment must be followed by a further separator, so the
/// trailing `.method` part always remains. Returns the end of the consumed
/// range (`start` when nothing matched).
fn generated_id_segments_end(name: &str, start: usize) -> usize {
    let mut end = start;
    loop {
        let Some(segment) = name[end..].strip_prefix(ID_SEPARATORS) else {
            return end;
        };
        let len = match segment.strip_prefix("0x") {
            Some(hex) => match hex.bytes().take_while(|b| b.is_ascii_hexdigit()).count() {
                0 => return end,
                n => 2 + n,
            },
            None => match segment.bytes().take_while(|b| b.is_ascii_digit()).count() {
                0 => return end,
                n => n,
            },
        };
        if !is_id_separator(segment.as_bytes().get(len)) {
            return end;
        }
        end += 1 + len;
    }
}

/// Strips the per-run identity segments from `LambdaForm` frames:
/// `java.lang.invoke.LambdaForm$MH+0x00007120c0658c00.74220245.invoke(...)` →
/// `java.lang.invoke.LambdaForm$MH.invoke(...)`. The hidden-class address and
/// counter depend on class-generation order and differ on every JVM run;
/// `.0x…`/`/0x…` renderings are handled the same.
fn strip_lambda_form_id(name: &str) -> Option<String> {
    let kind_start = name.find("LambdaForm$")? + "LambdaForm$".len();
    let kind = name[kind_start..].bytes().take_while(|b| b.is_ascii_alphanumeric()).count();
    let start = kind_start + kind;
    let end = generated_id_segments_end(name, start);
    (end > start).then(|| format!("{}{}", &name[..start], &name[end..]))
}

/// Strips the per-run identity segments from generated lambda classes:
/// `org.foo.Class$$Lambda$128+0x00007f20c013efc8.1766419680.run()` →
/// `org.foo.Class$$Lambda.run()`. The generation counter (`$128`) and the
/// hidden-class address (with its decimal rendering) depend on
/// class-generation order and differ on every JVM run.
fn strip_lambda_class_id(name: &str) -> Option<String> {
    let start = name.find("$$Lambda")? + "$$Lambda".len();
    let mut end = start;

    // Optional generation counter: `$123`, which must be followed by a separator.
    if let Some(counter) = name[end..].strip_prefix('$') {
        let digits = counter.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digits > 0 && is_id_separator(counter.as_bytes().get(digits)) {
            end += 1 + digits;
        }
    }

    let end = generated_id_segments_end(name, end);
    (end > start).then(|| format!("{}{}", &name[..start], &name[end..]))
}

/// Removes the digit run following `anchor` when it is followed by a `.` or
/// `/` separator (so only a whole name segment is ever affected).
fn strip_digits_after(name: &str, anchor: &str) -> Option<String> {
    let pos = name.find(anchor)? + anchor.len();
    let digits = name[pos..].bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || !matches!(name.as_bytes().get(pos + digits), Some(b'.') | Some(b'/')) {
        return None;
    }
    Some(format!("{}{}", &name[..pos], &name[pos + digits..]))
}

/// Strips the per-run numbering from JDK dynamic proxies:
/// `jdk.proxy4.$Proxy52.methodName()` → `jdk.proxy.$Proxy.methodName()`.
/// Both the proxy class counter and the `jdk.proxyN` module holding it are
/// allocated in load order and differ between runs.
fn strip_dynamic_proxy_id(name: &str) -> Option<String> {
    let module_stripped =
        strip_digits_after(name, "jdk.proxy").or_else(|| strip_digits_after(name, "jdk/proxy"));
    let current = module_stripped.as_deref().unwrap_or(name);
    strip_digits_after(current, "$Proxy").or(module_stripped)
}

/// Strips the generation counter from JDK reflection accessor classes:
/// `jdk.internal.reflect.GeneratedMethodAccessor42.invoke(...)` →
/// `jdk.internal.reflect.GeneratedMethodAccessor.invoke(...)`. An accessor
/// class is generated per reflected member in first-use order, so the counter
/// differs between runs.
fn strip_generated_accessor_id(name: &str) -> Option<String> {
    const ANCHORS: [&str; 3] = [
        "GeneratedMethodAccessor",
        "GeneratedConstructorAccessor",
        "GeneratedSerializationConstructorAccessor",
    ];
    ANCHORS.iter().find_map(|anchor| strip_digits_after(name, anchor))
}

/// Strips the line-number segment from JFR-converted frames:
/// `org.example.Foo.bar(int, int):42_[j]` → `org.example.Foo.bar(int, int)_[j]`.
/// The line number is where the sampled instruction sits within the method,
/// so it jitters from sample to sample and shifts whenever surrounding code
/// changes — it never identifies the method. `-1` marks an unknown line
/// (e.g. native or generated methods) and is stripped like any other.
fn strip_line_number(name: &str) -> Option<String> {
    let base_end = name.len() - frame_type(name).len();
    let base = &name[..base_end];
    let colon = base.rfind(':')?;
    let digits = base[colon + 1..].trim_start_matches('-');
    if !base[..colon].ends_with(')') || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}{}", &name[..colon], &name[base_end..]))
}

/// Applies all name-normalization rules to a frame name, in order.
/// Each rule returns `Some(new_name)` when it changed something.
fn simplify_name(name: &str) -> Cow<'_, str> {
    const RULES: [fn(&str) -> Option<String>; 6] = [
        strip_lambda_form_id,
        strip_lambda_class_id,
        strip_dynamic_proxy_id,
        strip_generated_accessor_id,
        strip_line_number,
        normalize_frame_type,
    ];
    let mut result = Cow::Borrowed(name);
    for rule in RULES {
        if let Some(simplified) = rule(&result) {
            result = Cow::Owned(simplified);
        }
    }
    result
}

/// Returns the raw frame-type suffix of a name (`"_[j]"`, `"_[i]"`, …),
/// or `""` when there is none.
fn frame_type(name: &str) -> &str {
    match name.rfind("_[") {
        Some(i) if name.ends_with(']') => &name[i..],
        _ => "",
    }
}

/// Converts a stack graph to a simplified form that removes insignificant
/// profile-to-profile differences between two JVM profiles so they diff
/// cleanly. Nodes that become identical after simplification are merged, with
/// their sample counts summed.
///
/// Rules applied (more will be added iteratively):
/// - Frame-type suffixes are normalized: `_[0]` (interpreted), `_[1]`
///   (C1-compiled), and `_[i]` (inlined) become `_[j]` (JIT-compiled),
///   merging frames that differ only by compilation state.
/// - `LambdaForm` frames lose their per-run identity segments:
///   `LambdaForm$MH+0x….74220245.invoke(...)` → `LambdaForm$MH.invoke(...)`.
/// - Generated lambda classes lose their per-run identity segments:
///   `Class$$Lambda$128+0x….1766419680.run()` → `Class$$Lambda.run()`.
/// - JDK dynamic proxies lose their per-run numbering:
///   `jdk.proxy4.$Proxy52.method()` → `jdk.proxy.$Proxy.method()`.
/// - Reflection accessors lose their generation counter:
///   `GeneratedMethodAccessor42.invoke(...)` → `GeneratedMethodAccessor.invoke(...)`
///   (likewise `GeneratedConstructorAccessor` and
///   `GeneratedSerializationConstructorAccessor`).
/// - JFR-converted frames lose their line number: `Foo.bar(int):42_[j]` →
///   `Foo.bar(int)_[j]`, merging samples taken at different points in a method.
/// - A same-named direct parent/child pair whose raw frame types differ is a
///   compilation-state artifact (inline record or tier transition) and folds
///   into one node that adopts the pair's children: `f_[j] -> f_[i]`,
///   `f_[i] -> f_[j]`, and `f_[1] -> f_[j]` all become a single `f_[j]`.
///   Identical raw types (`f_[j] -> f_[j]`) are genuine recursion and kept;
///   the fold is not transitive (at most one duplicate folds per node).
#[wasm_bindgen]
pub fn wasm_simplify_graph(
    children_offsets: &[i32],
    children_data: &[i32],
    names_data: &[u8],
    names_offsets: &[i32],
    values: &[i64],
    node_id: u32,
) -> WasmStackGraph {
    let node_id = node_id as usize;
    let raw_root_name = get_name(names_data, names_offsets, node_id);
    let root_name = simplify_name(raw_root_name);
    let mut new_graph = GraphBuilder::new(&root_name);
    new_graph.values[0] = values[node_id];

    // Stack entries: (original_node_id, new_parent_id, parent_frame_type, fold_used).
    // parent_frame_type is the raw frame-type suffix of the frame that produced
    // new_parent along this path; fold_used is true when a duplicate has already
    // been folded into new_parent along this path.
    let mut stack: Vec<(usize, usize, &str, bool)> = Vec::new();
    for &child in get_children(children_offsets, children_data, node_id) {
        stack.push((child as usize, 0, frame_type(raw_root_name), false));
    }

    while let Some((current, new_parent, parent_frame_type, fold_used)) = stack.pop() {
        let raw_name = get_name(names_data, names_offsets, current);
        let name = simplify_name(raw_name);
        let raw_type = frame_type(raw_name);

        // Compilation-state transitions record the same function twice as a
        // direct parent/child pair with differing raw frame types — a JIT frame
        // with its inline record (`f_[j] -> f_[i]`, `f_[i] -> f_[j]`) or a
        // tier boundary (`f_[1] -> f_[j]`) — where another profile has a single
        // `f_[j]`. Fold the duplicate away: its children become the parent's
        // own. The frame is dropped, not merged — the parent's inclusive value
        // already covers its samples. Same-named pairs with identical raw types
        // are genuine recursion and kept. At most one duplicate folds into a
        // node per path (fold_used): the fold is deliberately not transitive,
        // so e.g. `f_[j] -> f_[i] -> f_[j]` keeps its third frame rather than
        // swallowing what may be a real recursive call.
        if !fold_used && raw_type != parent_frame_type && name == new_graph.node_names[new_parent] {
            for &child in get_children(children_offsets, children_data, current) {
                stack.push((child as usize, new_parent, parent_frame_type, true));
            }
            continue;
        }

        let (new_current, _) = new_graph.get_or_create_child(new_parent, &name, values[current]);

        for &child in get_children(children_offsets, children_data, current) {
            stack.push((child as usize, new_current, raw_type, false));
        }
    }

    new_graph.into_wasm_stack_graph()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_stacks_impl;

    fn simplify(data: &[u8]) -> WasmStackGraph {
        let g = parse_stacks_impl(data);
        wasm_simplify_graph(
            &g.children_offsets,
            &g.children_data,
            &g.names_data,
            &g.names_offsets,
            &g.values,
            0,
        )
    }

    fn node_names(g: &WasmStackGraph) -> Vec<String> {
        (0..g.names_offsets.len().saturating_sub(1))
            .map(|i| {
                let start = g.names_offsets[i] as usize;
                let end = g.names_offsets[i + 1] as usize;
                String::from_utf8(g.names_data[start..end].to_vec()).unwrap()
            })
            .collect()
    }

    fn count(g: &WasmStackGraph, name: &str) -> usize {
        node_names(g).iter().filter(|n| n.as_str() == name).count()
    }

    fn value(g: &WasmStackGraph, name: &str) -> i64 {
        let names = node_names(g);
        let idx = names
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("{name} not found in {names:?}"));
        g.values[idx]
    }

    /// Every name-normalization rule, exercised over the frame formats produced
    /// by async-profiler (slash-separated, `_[x]`-suffixed) and by JFR
    /// conversion (dot-separated, `(params):line_[j]`).
    #[test]
    fn test_name_rules() {
        let cases: &[(&str, &str)] = &[
            // Frame-type normalization: _[0]/_[1]/_[i] → _[j].
            ("org/example/Foo.bar_[0]", "org/example/Foo.bar_[j]"),
            ("org/example/Foo.bar_[1]", "org/example/Foo.bar_[j]"),
            ("org/example/Foo.bar_[i]", "org/example/Foo.bar_[j]"),
            ("org/example/Foo.bar_[j]", "org/example/Foo.bar_[j]"),
            // Other frame types mark genuinely different code and are kept.
            ("sys_read_[k]", "sys_read_[k]"),
            ("kernel", "kernel"),
            // Line numbers, including -1 (unknown line).
            ("org.example.Foo.bar(int):10_[j]", "org.example.Foo.bar(int)_[j]"),
            ("org.example.Foo.bar(int):-1_[j]", "org.example.Foo.bar(int)_[j]"),
            ("org.example.Foo.bar():123", "org.example.Foo.bar()"),
            // A colon not preceded by ')' is not a line number.
            ("std::vector::push_back", "std::vector::push_back"),
            // LambdaForm per-run identity segments (JFR and async-profiler forms).
            (
                "java.lang.invoke.LambdaForm$MH+0x00007120c0658c00.74220245.invoke(java.lang.Object, java.lang.Object):-1_[j]",
                "java.lang.invoke.LambdaForm$MH.invoke(java.lang.Object, java.lang.Object)_[j]",
            ),
            (
                "java/lang/invoke/LambdaForm$DMH/0x0000000800c01c00.invokeStatic_[j]",
                "java/lang/invoke/LambdaForm$DMH.invokeStatic_[j]",
            ),
            // Generated lambda classes: counter and hidden-class address.
            (
                "org.gradle.launcher.daemon.server.DaemonStateCoordinator$$Lambda$128+0x00007f20c013efc8.1766419680.run():-1_[j]",
                "org.gradle.launcher.daemon.server.DaemonStateCoordinator$$Lambda.run()_[j]",
            ),
            (
                "org/foo/Class$$Lambda$123/0x0000000800c02420.run_[j]",
                "org/foo/Class$$Lambda.run_[j]",
            ),
            // Counter-only (older JDKs) and counter-less (JDK 21+) renderings.
            ("org.foo.Class$$Lambda$45.apply():5_[j]", "org.foo.Class$$Lambda.apply()_[j]"),
            (
                "org/foo/Class$$Lambda/0x0000000800c02420.accept_[j]",
                "org/foo/Class$$Lambda.accept_[j]",
            ),
            // JDK dynamic proxies: module and class numbering.
            ("jdk.proxy4.$Proxy52.query():-1_[j]", "jdk.proxy.$Proxy.query()_[j]"),
            ("jdk/proxy2/$Proxy17.invoke_[j]", "jdk/proxy/$Proxy.invoke_[j]"),
            // Reflection accessor generation counters.
            (
                "jdk.internal.reflect.GeneratedMethodAccessor42.invoke(java.lang.Object, java.lang.Object[]):-1_[j]",
                "jdk.internal.reflect.GeneratedMethodAccessor.invoke(java.lang.Object, java.lang.Object[])_[j]",
            ),
            (
                "jdk.internal.reflect.GeneratedConstructorAccessor7.newInstance(java.lang.Object[]):-1_[j]",
                "jdk.internal.reflect.GeneratedConstructorAccessor.newInstance(java.lang.Object[])_[j]",
            ),
            (
                "jdk.internal.reflect.GeneratedSerializationConstructorAccessor3.newInstance(java.lang.Object[]):-1_[j]",
                "jdk.internal.reflect.GeneratedSerializationConstructorAccessor.newInstance(java.lang.Object[])_[j]",
            ),
            // Rules compose: lambda id + line number + compilation tier.
            (
                "org.foo.Class$$Lambda$9+0xdead.123.get():42_[0]",
                "org.foo.Class$$Lambda.get()_[j]",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(simplify_name(input).as_ref(), *expected, "input: {input}");
        }
    }

    #[test]
    fn test_compilation_tiers_merge() {
        let g = simplify(b"org/ex/Foo.f_[0] 1\norg/ex/Foo.f_[1] 2\norg/ex/Foo.f_[j] 4\n");
        assert_eq!(count(&g, "org/ex/Foo.f_[j]"), 1, "{:?}", node_names(&g));
        assert_eq!(value(&g, "org/ex/Foo.f_[j]"), 7);
    }

    #[test]
    fn test_line_number_variants_merge() {
        let g = simplify(b"org.ex.Foo.f(int):10_[j] 3\norg.ex.Foo.f(int):20_[j] 4\n");
        assert_eq!(count(&g, "org.ex.Foo.f(int)_[j]"), 1, "{:?}", node_names(&g));
        assert_eq!(value(&g, "org.ex.Foo.f(int)_[j]"), 7);
    }

    #[test]
    fn test_lambda_form_addresses_merge() {
        let g = simplify(
            b"java/lang/invoke/LambdaForm$MH/0x00000001.invokeBasic_[j] 3\n\
              java/lang/invoke/LambdaForm$MH/0x00000002.invokeBasic_[j] 4\n",
        );
        assert_eq!(count(&g, "java/lang/invoke/LambdaForm$MH.invokeBasic_[j]"), 1);
        assert_eq!(value(&g, "java/lang/invoke/LambdaForm$MH.invokeBasic_[j]"), 7);
    }

    #[test]
    fn test_lambda_classes_merge() {
        let g = simplify(
            b"org.foo.Class$$Lambda$1+0x00000a.111.run():-1_[j] 3\n\
              org.foo.Class$$Lambda$2+0x00000b.222.run():-1_[j] 4\n",
        );
        assert_eq!(count(&g, "org.foo.Class$$Lambda.run()_[j]"), 1);
        assert_eq!(value(&g, "org.foo.Class$$Lambda.run()_[j]"), 7);
    }

    #[test]
    fn test_proxies_merge() {
        let g = simplify(b"jdk.proxy3.$Proxy41.query():-1_[j] 3\njdk.proxy4.$Proxy52.query():-1_[j] 4\n");
        assert_eq!(count(&g, "jdk.proxy.$Proxy.query()_[j]"), 1);
        assert_eq!(value(&g, "jdk.proxy.$Proxy.query()_[j]"), 7);
    }

    #[test]
    fn test_generated_accessors_merge() {
        let g = simplify(
            b"jdk.internal.reflect.GeneratedMethodAccessor17.invoke(java.lang.Object, java.lang.Object[]):-1_[j] 3\n\
              jdk.internal.reflect.GeneratedMethodAccessor42.invoke(java.lang.Object, java.lang.Object[]):-1_[j] 4\n",
        );
        let expected = "jdk.internal.reflect.GeneratedMethodAccessor.invoke(java.lang.Object, java.lang.Object[])_[j]";
        assert_eq!(count(&g, expected), 1);
        assert_eq!(value(&g, expected), 7);
    }

    #[test]
    fn test_inlined_duplicate_folds_into_parent() {
        let g = simplify(b"org/ex/Foo.f_[j];org/ex/Foo.f_[i];org/ex/Bar.g_[j] 5\n");
        assert_eq!(count(&g, "org/ex/Foo.f_[j]"), 1, "{:?}", node_names(&g));
        // The duplicate is dropped, not merged — no double counting.
        assert_eq!(value(&g, "org/ex/Foo.f_[j]"), 5);
        assert_eq!(value(&g, "org/ex/Bar.g_[j]"), 5);
    }

    #[test]
    fn test_inlined_duplicate_folds_in_either_direction() {
        let g = simplify(b"org/ex/Foo.f_[i];org/ex/Foo.f_[j];org/ex/Bar.g_[j] 5\n");
        assert_eq!(count(&g, "org/ex/Foo.f_[j]"), 1, "{:?}", node_names(&g));
        assert_eq!(value(&g, "org/ex/Bar.g_[j]"), 5);
    }

    #[test]
    fn test_tier_transition_duplicate_folds() {
        let g = simplify(b"org/ex/Foo.f_[1];org/ex/Foo.f_[j];org/ex/Bar.g_[j] 5\n");
        assert_eq!(count(&g, "org/ex/Foo.f_[j]"), 1, "{:?}", node_names(&g));
    }

    #[test]
    fn test_recursion_is_kept() {
        // Same-named pairs with identical raw frame types are genuine recursion.
        let g = simplify(b"org/ex/Foo.f_[j];org/ex/Foo.f_[j] 5\n");
        assert_eq!(count(&g, "org/ex/Foo.f_[j]"), 2, "{:?}", node_names(&g));
    }

    #[test]
    fn test_duplicate_fold_is_not_transitive() {
        // Only one duplicate folds per node: the third frame may be a real
        // recursive call and is kept.
        let g = simplify(b"org/ex/Foo.f_[j];org/ex/Foo.f_[i];org/ex/Foo.f_[j] 5\n");
        assert_eq!(count(&g, "org/ex/Foo.f_[j]"), 2, "{:?}", node_names(&g));
    }

    #[test]
    fn test_inlined_and_uninlined_paths_merge() {
        // Warm samples record f_[j] -> f_[i] -> x, cold samples f_[j] -> x.
        // After the fold both shapes merge into f -> x.
        let g = simplify(
            b"org/ex/Foo.f_[j];org/ex/Foo.f_[i];org/ex/Bar.x_[j] 3\n\
              org/ex/Foo.f_[j];org/ex/Bar.x_[j] 4\n",
        );
        assert_eq!(count(&g, "org/ex/Bar.x_[j]"), 1, "{:?}", node_names(&g));
        assert_eq!(value(&g, "org/ex/Bar.x_[j]"), 7);
    }

    #[test]
    fn test_duplicate_fold_uses_simplified_names() {
        // JFR inline pair with differing line numbers still folds: names are
        // compared after line-number stripping, raw frame types before.
        let g = simplify(b"a.B.f(int):10_[j];a.B.f(int):55_[i];a.B.g():3_[j] 5\n");
        assert_eq!(count(&g, "a.B.f(int)_[j]"), 1, "{:?}", node_names(&g));
        assert_eq!(value(&g, "a.B.g()_[j]"), 5);
    }
}
