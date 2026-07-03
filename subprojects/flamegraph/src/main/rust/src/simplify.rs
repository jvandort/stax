use wasm_bindgen::prelude::*;

use crate::builder::{format_jfr_name, parse_method_name, ParsedName};
use crate::{get_children, get_name, GraphBuilder, WasmStackGraph};

/// Strips trailing digit suffixes from generated accessor simple class names.
/// `"GeneratedMethodAccessor42"` → `"GeneratedMethodAccessor"`, others unchanged.
fn strip_accessor_digits(simple_class: &str) -> &str {
    const GENERATED_PREFIXES: &[&str] = &[
        "GeneratedMethodAccessor",
        "GeneratedSerializationConstructorAccessor",
        "GeneratedConstructorAccessor",
    ];
    for prefix in GENERATED_PREFIXES {
        if let Some(rest) = simple_class.strip_prefix(prefix) {
            if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
                return prefix;
            }
        }
    }
    simple_class
}

/// Normalizes reflection accessor frames using the parsed name fields directly:
///
/// - Strips digit suffixes from generated accessor `simple_class_name`s.
/// - Maps accessor implementation `simple_class_name`s to canonical names,
///   dropping the package prefix (e.g. `NativeMethodAccessorImpl` → `MethodAccessor`).
/// - Renames internal method variants so the same-name-as-parent collapse rule
///   can elide delegation/trampoline chains without hardcoded skips:
///   - `MethodAccessor.invoke0`           → `MethodAccessor.invoke`
///   - `ConstructorAccessor.newInstance0` → `ConstructorAccessor.newInstance`
///   - `Constructor.newInstanceWithCaller` → `Constructor.newInstance`
fn normalize_parsed_name(p: &ParsedName) -> String {
    // Step 1: Strip numeric suffixes from generated accessor simple class names.
    let simple_class = strip_accessor_digits(p.simple_class_name());

    // Step 2: Map simple class name to a canonical package-free name.
    const CLASS_MAPPINGS: &[(&str, &str)] = &[
        ("GeneratedMethodAccessor",           "MethodAccessor"),
        ("NativeMethodAccessorImpl",          "MethodAccessor"),
        ("DelegatingMethodAccessorImpl",      "MethodAccessor"),
        ("GeneratedConstructorAccessor",      "ConstructorAccessor"),
        ("NativeConstructorAccessorImpl",     "ConstructorAccessor"),
        ("DelegatingConstructorAccessorImpl", "ConstructorAccessor"),
    ];
    let canonical_class = CLASS_MAPPINGS.iter()
        .find(|&&(from, _)| from == simple_class)
        .map(|&(_, to)| to);

    // Step 3: Rename internal method variants, keyed on (canonical_or_simple_class, method_name).
    let class_key = canonical_class.unwrap_or(simple_class);
    const METHOD_RENAMES: &[(&str, &str, &str)] = &[
        ("MethodAccessor",     "invoke0",               "invoke"),
        ("ConstructorAccessor","newInstance0",           "newInstance"),
        ("Constructor",        "newInstanceWithCaller",  "newInstance"),
    ];
    let method = METHOD_RENAMES.iter()
        .find(|&&(cls, m, _)| cls == class_key && m == p.method_name())
        .map(|&(_, _, to)| to)
        .unwrap_or(p.method_name());

    // Step 4: Format the result.
    // Use format_jfr_name to get the fully-formatted string with simplified params,
    // then rebuild the class.method prefix if anything changed.
    let formatted = format_jfr_name(p, true, true, false);
    let paren_pos = formatted.find('(').unwrap_or(formatted.len());
    let params_suffix = &formatted[paren_pos..];

    if let Some(class) = canonical_class {
        // Canonical (package-free) class: "MethodAccessor.invoke(params)"
        let mut s = String::with_capacity(class.len() + 1 + method.len() + params_suffix.len());
        s.push_str(class);
        s.push('.');
        s.push_str(method);
        s.push_str(params_suffix);
        return s;
    }

    if method != p.method_name() || simple_class != p.simple_class_name() {
        // The method name or simple class changed — rebuild the simple_class.method segment.
        // formatted = "package.OldSimpleClass.originalMethod(params)"
        // method ends at paren_pos, starts at paren_pos - original_method_name.len().
        // OldSimpleClass ends at method_start - 1, starts at method_start - 1 - simple_class_name.len().
        let method_start = paren_pos - p.method_name().len();
        let simple_class_start = method_start.saturating_sub(1 + p.simple_class_name().len());
        let mut s = String::with_capacity(
            simple_class_start + simple_class.len() + 1 + method.len() + params_suffix.len(),
        );
        s.push_str(&formatted[..simple_class_start]);
        s.push_str(simple_class);
        s.push('.');
        s.push_str(method);
        s.push_str(params_suffix);
        return s;
    }

    formatted
}

/// Returns the `class.method` prefix of a normalized frame name, stripping the
/// parameter list.  Used for same-name-as-parent deduplication so that internal
/// JDK method variants with different signatures (e.g. `invoke0(Object, Object[], Class)`
/// vs `invoke(Object, Object[])`) still compare as equal after class+method renaming.
fn method_id(name: &str) -> &str {
    name.find('(').map(|i| &name[..i]).unwrap_or(name)
}

/// Returns true for frames that carry no useful information and should be elided
/// from the output graph, with their children re-parented to the nearest ancestor.
///
/// Delegation and trampoline chains (e.g. DelegatingX → NativeX → NativeX.op0)
/// are handled instead by `normalize_accessor_class` + the same-name-as-parent
/// collapse rule in `wasm_simplify_graph`, so only truly content-free frames
/// belong here.
///
/// `[stale_jmethodID]` appears when async-profiler captured a jmethodID for a
/// dynamically-generated class that the JVM has since unloaded.  The frame
/// carries no useful information and is dropped.
fn should_skip_node(name: &str) -> bool {
    name.contains("[stale_jmethodID]")
}

/// Returns true when the raw frame name belongs to JVM method-handle plumbing
/// that should be collapsed into a single `[MethodHandle]` node.
///
/// This covers the various internal `java.lang.invoke` implementation classes
/// that appear in the call stack depending on JVM warmup state:
/// - Cold: `DelegatingMethodHandle` → `LambdaForm` chain
/// - Warm: direct `LambdaForm` invocation
/// - JIT-compiled paths through `BoundMethodHandle`, `DirectMethodHandle`, etc.
fn is_method_handle_infra(name: &str) -> bool {
    (name.contains("java.lang.invoke") || name.contains("java/lang/invoke"))
        && (name.contains("LambdaForm")
            || (name.contains("MethodHandle") && !name.contains("MethodHandles."))
            || name.contains("Invokers")
            || name.contains("CallSite"))
}

/// Produces a canonical simplified node name in JFR format, stripping line
/// numbers, async-profiler frame types, dynamic generated-class suffixes, and
/// normalizing accessor implementations to a common name.
/// Unrecognized names are returned unchanged.
fn simplify_node_name(name: &str) -> String {
    match parse_method_name(name) {
        Some(p) => normalize_parsed_name(&p),
        None => name.to_string(),
    }
}

/// Converts a stack graph to a simplified form where:
/// - Both JFR-based and async-profiler-based method names are normalized to JFR format.
/// - Line numbers and async-profiler frame types (`_[x]`) are stripped.
/// - Nodes that become identical after simplification (e.g., same method at different
///   line numbers) are merged, with their sample counts summed.
/// - Generated and native reflection accessor frames are unified as `MethodAccessor`.
/// - `NativeMethodAccessorImpl.invoke0` is elided so the two-frame native path
///   collapses to a single `MethodAccessor.invoke` node.
/// - All `java.lang.invoke` method-handle infrastructure frames are collapsed into a
///   single `[MethodHandle]` node, regardless of warmup state or path depth.
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
    let root_name = get_name(names_data, names_offsets, node_id);
    let simplified_root = simplify_node_name(root_name);
    let mut new_graph = GraphBuilder::new(&simplified_root);
    new_graph.values[0] = values[node_id];

    // Stack entries: (original_node_id, new_parent_id, inside_mh)
    // inside_mh = Some(mh_node_id) when we are traversing a method-handle
    //             infrastructure chain; the first non-MH frame will be parented
    //             onto that node and the chain ends.
    //           = None for normal processing.
    let mut stack: Vec<(usize, usize, Option<usize>)> = Vec::new();
    for &child in get_children(children_offsets, children_data, node_id) {
        stack.push((child as usize, 0, None));
    }

    while let Some((current, new_parent, inside_mh)) = stack.pop() {
        let name = get_name(names_data, names_offsets, current);

        if let Some(mh_id) = inside_mh {
            if is_method_handle_infra(name) || should_skip_node(name) {
                // Still inside the MH chain — drop this frame and continue.
                for &child in get_children(children_offsets, children_data, current) {
                    stack.push((child as usize, new_parent, Some(mh_id)));
                }
            } else {
                // First frame outside the MH chain — parent it onto [MethodHandle].
                let simplified = simplify_node_name(name);
                let (new_current, _) = new_graph.get_or_create_child(mh_id, &simplified, values[current]);
                for &child in get_children(children_offsets, children_data, current) {
                    stack.push((child as usize, new_current, None));
                }
            }
            continue;
        }

        if should_skip_node(name) {
            // Re-parent children directly onto the nearest surviving ancestor.
            for &child in get_children(children_offsets, children_data, current) {
                stack.push((child as usize, new_parent, None));
            }
            continue;
        }

        if is_method_handle_infra(name) {

            // Create a single [MethodHandle] placeholder under the current parent
            // and traverse the entire infrastructure chain under it.
            let (mh_id, _) = new_graph.get_or_create_child(new_parent, "[MethodHandle]", values[current]);
            for &child in get_children(children_offsets, children_data, current) {
                stack.push((child as usize, new_parent, Some(mh_id)));
            }
            continue;
        }

        let simplified = simplify_node_name(name);

        // Skip this frame if its normalized class.method (ignoring parameter lists)
        // matches the parent's.  Combined with the method renames in
        // normalize_accessor_class, this collapses entire JDK reflection delegation
        // chains into a single canonical node:
        //   DelegatingX.m(args) → NativeX.m(args)   — same method_id → skip
        //   NativeX.m0(args')                        — renamed to m → same method_id → skip
        //   Constructor.newInstanceWithCaller(args')  — renamed to newInstance → same → skip
        if method_id(&simplified) == method_id(&new_graph.node_names[new_parent]) {
            for &child in get_children(children_offsets, children_data, current) {
                stack.push((child as usize, new_parent, None));
            }
            continue;
        }

        let (new_current, _) = new_graph.get_or_create_child(new_parent, &simplified, values[current]);

        for &child in get_children(children_offsets, children_data, current) {
            stack.push((child as usize, new_current, None));
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

    #[test]
    fn test_jfr_strips_line_number() {
        let g = simplify(b"org.example.Foo.bar(int):10_[j] 5\n");
        let names = node_names(&g);
        assert!(names.contains(&"org.example.Foo.bar(int)".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n.contains(":10")), "{names:?}");
    }

    #[test]
    fn test_async_profiler_converts_to_jfr() {
        let g = simplify(b"org/example/Foo.bar_[j] 5\n");
        let names = node_names(&g);
        assert!(names.contains(&"org.example.Foo.bar()".to_string()), "{names:?}");
    }

    #[test]
    fn test_merges_nodes_with_different_line_numbers() {
        // Two stacks through the same method at different line numbers should merge.
        let g = simplify(
            b"org.example.Foo.bar(int):10_[j] 3\norg.example.Foo.bar(int):20_[j] 4\n",
        );
        let names = node_names(&g);
        let bar_count = names.iter().filter(|n| n.as_str() == "org.example.Foo.bar(int)").count();
        assert_eq!(bar_count, 1, "expected exactly one merged bar node, got {names:?}");

        let bar_idx = names.iter().position(|n| n == "org.example.Foo.bar(int)").unwrap();
        assert_eq!(g.values[bar_idx], 7);
    }

    #[test]
    fn test_non_method_names_unchanged() {
        let g = simplify(b"kernel 5\n");
        let names = node_names(&g);
        assert!(names.contains(&"kernel".to_string()), "{names:?}");
    }

    #[test]
    fn test_generated_method_accessor_normalizes_to_method_accessor() {
        let g = simplify(b"jdk.internal.reflect.GeneratedMethodAccessor42.invoke(java.lang.Object[]):10_[j] 5\n");
        let names = node_names(&g);
        assert!(
            names.iter().any(|n| n.contains("MethodAccessor.invoke")),
            "expected MethodAccessor, got {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("GeneratedMethodAccessor")),
            "expected GeneratedMethodAccessor replaced, got {names:?}"
        );
    }

    #[test]
    fn test_generated_method_accessors_merge() {
        let g = simplify(
            b"jdk.internal.reflect.GeneratedMethodAccessor42.invoke(java.lang.Object[]):10_[j] 3\n\
              jdk.internal.reflect.GeneratedMethodAccessor17.invoke(java.lang.Object[]):10_[j] 4\n",
        );
        let names = node_names(&g);
        let count = names.iter().filter(|n| n.contains("MethodAccessor.invoke")).count();
        assert_eq!(count, 1, "expected merged into one node, got {names:?}");
        let idx = names.iter().position(|n| n.contains("MethodAccessor.invoke")).unwrap();
        assert_eq!(g.values[idx], 7, "expected merged sample count");
    }

    #[test]
    fn test_native_method_accessor_normalizes_to_method_accessor() {
        let g = simplify(b"jdk.internal.reflect.NativeMethodAccessorImpl.invoke(java.lang.Object, java.lang.Object[]):10_[j] 5\n");
        let names = node_names(&g);
        assert!(
            names.iter().any(|n| n.contains("MethodAccessor.invoke")),
            "expected MethodAccessor, got {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("NativeMethodAccessorImpl")),
            "expected NativeMethodAccessorImpl replaced, got {names:?}"
        );
    }

    #[test]
    fn test_native_invoke0_is_skipped() {
        // invoke0 should be elided; its child (target) re-parents onto invoke.
        let g = simplify(
            b"jdk.internal.reflect.NativeMethodAccessorImpl.invoke(java.lang.Object, java.lang.Object[]):10_[j];\
              jdk.internal.reflect.NativeMethodAccessorImpl.invoke0(java.lang.Object, java.lang.Object[], java.lang.Class):1_[j];\
              org.example.Target.run():5_[j] 5\n",
        );
        let names = node_names(&g);
        assert!(!names.iter().any(|n| n.contains("invoke0")), "invoke0 should be skipped, got {names:?}");
        assert!(names.iter().any(|n| n.contains("Target.run")), "target should still appear, got {names:?}");
    }

    #[test]
    fn test_native_and_generated_accessor_paths_merge() {
        // Native path (two frames) and generated path (one frame) should
        // collapse to the same MethodAccessor.invoke node and merge.
        let g = simplify(
            b"jdk.internal.reflect.NativeMethodAccessorImpl.invoke(java.lang.Object, java.lang.Object[]):10_[j];\
              jdk.internal.reflect.NativeMethodAccessorImpl.invoke0(java.lang.Object, java.lang.Object[], java.lang.Class):1_[j];\
              org.example.Target.run():5_[j] 3\n\
              jdk.internal.reflect.GeneratedMethodAccessor42.invoke(java.lang.Object, java.lang.Object[]):10_[j];\
              org.example.Target.run():5_[j] 4\n",
        );
        let names = node_names(&g);
        let count = names.iter().filter(|n| n.contains("MethodAccessor.invoke")).count();
        assert_eq!(count, 1, "expected one merged MethodAccessor node, got {names:?}");
        let idx = names.iter().position(|n| n.contains("MethodAccessor.invoke")).unwrap();
        assert_eq!(g.values[idx], 7, "expected merged sample count of 3+4=7");
    }

    #[test]
    fn test_generated_constructor_accessor_normalizes_to_constructor_accessor() {
        let g = simplify(b"jdk.internal.reflect.GeneratedConstructorAccessor12.newInstance(java.lang.Object[]):5_[j] 5\n");
        let names = node_names(&g);
        assert!(names.iter().any(|n| n.contains("ConstructorAccessor.newInstance")), "expected ConstructorAccessor, got {names:?}");
        assert!(!names.iter().any(|n| n.contains("GeneratedConstructorAccessor")), "expected GeneratedConstructorAccessor replaced, got {names:?}");
    }

    #[test]
    fn test_native_constructor_accessor_normalizes_to_constructor_accessor() {
        let g = simplify(b"jdk.internal.reflect.NativeConstructorAccessorImpl.newInstance(java.lang.Object[]):5_[j] 5\n");
        let names = node_names(&g);
        assert!(names.iter().any(|n| n.contains("ConstructorAccessor.newInstance")), "expected ConstructorAccessor, got {names:?}");
        assert!(!names.iter().any(|n| n.contains("NativeConstructorAccessorImpl")), "expected NativeConstructorAccessorImpl replaced, got {names:?}");
    }

    #[test]
    fn test_delegating_constructor_accessor_normalizes_to_constructor_accessor() {
        let g = simplify(b"jdk.internal.reflect.DelegatingConstructorAccessorImpl.newInstance(java.lang.Object[]):5_[j] 5\n");
        let names = node_names(&g);
        assert!(names.iter().any(|n| n.contains("ConstructorAccessor.newInstance")), "expected ConstructorAccessor, got {names:?}");
        assert!(!names.iter().any(|n| n.contains("DelegatingConstructorAccessorImpl")), "expected DelegatingConstructorAccessorImpl replaced, got {names:?}");
    }

    #[test]
    fn test_native_constructor_newinstance0_is_skipped() {
        let g = simplify(
            b"jdk.internal.reflect.NativeConstructorAccessorImpl.newInstance(java.lang.Object[]):5_[j];\
              jdk.internal.reflect.NativeConstructorAccessorImpl.newInstance0(java.lang.reflect.Constructor, java.lang.Object[]):1_[j];\
              org.example.Target.<init>():3_[j] 5\n",
        );
        let names = node_names(&g);
        assert!(!names.iter().any(|n| n.contains("newInstance0")), "newInstance0 should be skipped, got {names:?}");
        assert!(names.iter().any(|n| n.contains("Target.<init>")), "target should appear, got {names:?}");
    }

    #[test]
    fn test_constructor_new_instance_with_caller_is_collapsed() {
        // JDK 17+ warm path: Constructor.newInstance → Constructor.newInstanceWithCaller → accessor → target
        // Cold path:          Constructor.newInstance → accessor → target
        // newInstanceWithCaller is renamed to newInstance so the same-name rule collapses it.
        let g = simplify(
            b"org.example.Caller.go():1_[j];\
              java.lang.reflect.Constructor.newInstance(java.lang.Object[]):1_[j];\
              java.lang.reflect.Constructor.newInstanceWithCaller(java.lang.Object[], boolean, java.lang.Class):1_[j];\
              jdk.internal.reflect.GeneratedConstructorAccessor12.newInstance(java.lang.Object[]):1_[j];\
              org.example.Target.<init>():3_[j] 4\n\
              org.example.Caller.go():1_[j];\
              java.lang.reflect.Constructor.newInstance(java.lang.Object[]):1_[j];\
              jdk.internal.reflect.DelegatingConstructorAccessorImpl.newInstance(java.lang.Object[]):1_[j];\
              org.example.Target.<init>():3_[j] 3\n",
        );
        let names = node_names(&g);
        assert!(!names.iter().any(|n| n.contains("newInstanceWithCaller")), "newInstanceWithCaller should be collapsed, got {names:?}");
        let target_count = names.iter().filter(|n| n.contains("Target.<init>")).count();
        assert_eq!(target_count, 1, "expected one merged Target node, got {names:?}");
        let target_idx = names.iter().position(|n| n.contains("Target.<init>")).unwrap();
        assert_eq!(g.values[target_idx], 7, "expected merged sample count 4+3=7");
    }

    #[test]
    fn test_all_constructor_accessor_paths_merge() {
        // Native (two frames), generated (one frame), and delegating (one frame) paths should
        // all collapse to the same ConstructorAccessor.newInstance node.
        let g = simplify(
            b"jdk.internal.reflect.NativeConstructorAccessorImpl.newInstance(java.lang.Object[]):5_[j];\
              jdk.internal.reflect.NativeConstructorAccessorImpl.newInstance0(java.lang.reflect.Constructor, java.lang.Object[]):1_[j];\
              org.example.Target.<init>():3_[j] 3\n\
              jdk.internal.reflect.GeneratedConstructorAccessor12.newInstance(java.lang.Object[]):5_[j];\
              org.example.Target.<init>():3_[j] 4\n\
              jdk.internal.reflect.DelegatingConstructorAccessorImpl.newInstance(java.lang.Object[]):5_[j];\
              org.example.Target.<init>():3_[j] 2\n",
        );
        let names = node_names(&g);
        let count = names.iter().filter(|n| n.contains("ConstructorAccessor.newInstance")).count();
        assert_eq!(count, 1, "expected one merged ConstructorAccessor node, got {names:?}");
        let idx = names.iter().position(|n| n.contains("ConstructorAccessor.newInstance")).unwrap();
        assert_eq!(g.values[idx], 9, "expected merged sample count of 3+4+2=9");
    }

    #[test]
    fn test_generated_serialization_constructor_accessor_strips_number() {
        let g = simplify(b"jdk.internal.reflect.GeneratedSerializationConstructorAccessor3.newInstance(java.lang.Object[]):5_[j] 5\n");
        let names = node_names(&g);
        assert!(
            names.iter().any(|n| n.contains("GeneratedSerializationConstructorAccessor.newInstance")),
            "expected stripped name, got {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("GeneratedSerializationConstructorAccessor3")),
            "expected number stripped, got {names:?}"
        );
    }

    #[test]
    fn test_lambda_form_collapses_to_method_handle() {
        // LambdaForm is MH infrastructure — it should collapse to [MethodHandle], not appear by name.
        let g = simplify(b"java/lang/invoke/LambdaForm$MH/0x00000001.invokeBasic_[j] 5\n");
        let names = node_names(&g);
        assert!(names.iter().any(|n| n == "[MethodHandle]"), "expected [MethodHandle], got {names:?}");
        assert!(!names.iter().any(|n| n.contains("LambdaForm")), "LambdaForm should be collapsed, got {names:?}");
    }

    #[test]
    fn test_lambda_form_variants_collapse_to_single_method_handle() {
        // Multiple different LambdaForm addresses should produce a single [MethodHandle] node.
        let g = simplify(
            b"java/lang/invoke/LambdaForm$MH/0x00000001.invokeBasic_[j] 3\n\
              java/lang/invoke/LambdaForm$MH/0x00000002.invokeBasic_[j] 4\n",
        );
        let names = node_names(&g);
        let mh_count = names.iter().filter(|n| n.as_str() == "[MethodHandle]").count();
        assert_eq!(mh_count, 1, "expected single [MethodHandle] node, got {names:?}");
        assert!(!names.iter().any(|n| n.contains("LambdaForm")), "LambdaForm should be collapsed, got {names:?}");
        let idx = names.iter().position(|n| n.as_str() == "[MethodHandle]").unwrap();
        assert_eq!(g.values[idx], 7, "expected merged sample count");
    }

    #[test]
    fn test_lambda_form_dot_separated_collapses_to_method_handle() {
        // JFR-style LambdaForm names (dot-separated) should also collapse to [MethodHandle].
        let g = simplify(
            b"java.lang.invoke.LambdaForm$MH.0x00000001.invoke():10_[j] 3\n\
              java.lang.invoke.LambdaForm$MH.0x00000002.invoke():10_[j] 4\n",
        );
        let names = node_names(&g);
        let mh_count = names.iter().filter(|n| n.as_str() == "[MethodHandle]").count();
        assert_eq!(mh_count, 1, "expected single [MethodHandle] node, got {names:?}");
        assert!(!names.iter().any(|n| n.contains("LambdaForm")), "LambdaForm should be collapsed, got {names:?}");
        let idx = names.iter().position(|n| n.as_str() == "[MethodHandle]").unwrap();
        assert_eq!(g.values[idx], 7, "expected merged sample count");
    }

    #[test]
    fn test_direct_lambda_form_collapses_to_method_handle() {
        // Warm path: direct LambdaForm invocation.
        let g = simplify(
            b"org.example.Caller.call():1_[j];\
              java.lang.invoke.LambdaForm$MH.0x00000001.invokeBasic_[j];\
              org.example.Target.run():5_[j] 5\n",
        );
        let names = node_names(&g);
        assert!(names.iter().any(|n| n == "[MethodHandle]"), "expected [MethodHandle] node, got {names:?}");
        assert!(!names.iter().any(|n| n.contains("LambdaForm")), "LambdaForm should be collapsed, got {names:?}");
        assert!(names.iter().any(|n| n.contains("Target.run")), "target should appear, got {names:?}");
    }

    #[test]
    fn test_delegating_method_handle_chain_collapses_to_method_handle() {
        // Cold path: DelegatingMethodHandle → LambdaForm chain.
        let g = simplify(
            b"org.example.Caller.call():1_[j];\
              java.lang.invoke.DelegatingMethodHandle.invoke():1_[j];\
              java.lang.invoke.LambdaForm$MH.0x00000001.invokeBasic_[j];\
              org.example.Target.run():5_[j] 5\n",
        );
        let names = node_names(&g);
        assert!(names.iter().any(|n| n == "[MethodHandle]"), "expected [MethodHandle] node, got {names:?}");
        assert!(!names.iter().any(|n| n.contains("DelegatingMethodHandle")), "DelegatingMethodHandle should be collapsed");
        assert!(!names.iter().any(|n| n.contains("LambdaForm")), "LambdaForm should be collapsed");
        assert!(names.iter().any(|n| n.contains("Target.run")), "target should appear, got {names:?}");
    }

    #[test]
    fn test_different_mh_paths_produce_single_method_handle_node() {
        // Cold path (DelegatingMethodHandle → LambdaForm) and warm path (direct LambdaForm)
        // through the same caller → target should both collapse to the same [MethodHandle] node.
        let g = simplify(
            b"org.example.Caller.call():1_[j];\
              java.lang.invoke.DelegatingMethodHandle.invoke():1_[j];\
              java.lang.invoke.LambdaForm$MH.0x00000001.invokeBasic_[j];\
              org.example.Target.run():5_[j] 3\n\
              org.example.Caller.call():1_[j];\
              java.lang.invoke.LambdaForm$MH.0x00000002.invokeBasic_[j];\
              org.example.Target.run():5_[j] 4\n",
        );
        let names = node_names(&g);
        let mh_count = names.iter().filter(|n| n.as_str() == "[MethodHandle]").count();
        assert_eq!(mh_count, 1, "expected single [MethodHandle] node, got {names:?}");
        let target_count = names.iter().filter(|n| n.contains("Target.run")).count();
        assert_eq!(target_count, 1, "expected merged Target.run node, got {names:?}");
        let target_idx = names.iter().position(|n| n.contains("Target.run")).unwrap();
        assert_eq!(g.values[target_idx], 7, "expected merged sample count 3+4=7");
    }
}
