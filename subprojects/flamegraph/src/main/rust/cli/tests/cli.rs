// End-to-end tests: drive the real binary against a daemon in an isolated home directory.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const STACKS: &str = "a;b;c 5\na;b;d 3\na;e 2\nx;b;c 4\n";

/// A daemon in its own `HOME`, started by the first `load` and stopped on drop.
struct Daemon {
    home: PathBuf,
}

impl Daemon {
    fn start(name: &str) -> Daemon {
        let home = std::env::temp_dir().join(format!("stax-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        Daemon { home }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_stax"));
        cmd.args(args).env("HOME", &self.home).env_remove("XDG_RUNTIME_DIR").current_dir(&self.home);
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// Runs a command that must succeed, returning its stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert_eq!(out.status.code(), Some(0), "{args:?} failed: {}", stderr(&out));
        stdout(&out)
    }

    /// Runs a command that must fail with exit code 1, returning its stderr.
    fn fail(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert_eq!(out.status.code(), Some(1), "{args:?} should fail: {}", stdout(&out));
        stderr(&out)
    }

    fn write(&self, name: &str, data: &[u8]) -> PathBuf {
        let path = self.home.join(name);
        std::fs::write(&path, data).unwrap();
        path
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.run(&["shutdown"]);
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn jfr_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../test-data/cpu-sample.jfr")
}

#[test]
fn guide() {
    let daemon = Daemon::start("guide");
    let guide = daemon.ok(&[]);
    assert!(guide.starts_with("stax: explore profiles from the shell."), "{guide}");
    for args in [&["-h"][..], &["--help"], &["help"]] {
        assert_eq!(daemon.ok(args), guide, "{args:?}");
    }
    // Every command and every long option is in it.
    let help = daemon.ok(&["help", "load"]);
    assert!(help.contains("Profile files"), "{help}");
    for command in [
        "load", "list", "unload", "shutdown", "top", "search", "fn", "calls", "callers", "subtree", "children",
        "node", "raw-name", "aggregate", "icicle", "diff",
    ] {
        assert!(guide.contains(&format!("  stax {command} ")) || guide.contains(&format!("  stax {command}\n")), "{command}");
        let help = daemon.ok(&["help", command]);
        for option in help.split_whitespace().filter(|w| w.starts_with("--") && *w != "--help") {
            let option = option.trim_end_matches(|c: char| !c.is_alphanumeric());
            assert!(guide.contains(option), "{command} {option} is not in the guide");
        }
    }
}

#[test]
fn lifecycle() {
    let daemon = Daemon::start("lifecycle");
    assert!(daemon.fail(&["list"]).contains("No stax daemon is running"));
    daemon.write("app-stacks.txt", STACKS.as_bytes());
    // `load` starts the daemon, saying so on stderr.
    let out = daemon.run(&["load", "app-stacks.txt"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).starts_with("Started the stax daemon"), "{}", stderr(&out));
    let out = daemon.run(&["load", "app-stacks.txt"]);
    assert_eq!(stderr(&out), "");
    assert!(daemon.ok(&["shutdown"]).contains("stopped"));
    assert!(daemon.fail(&["shutdown"]).contains("No stax daemon is running"));
    // A stale socket from the stopped daemon doesn't block a new one.
    assert!(daemon.ok(&["load", "app-stacks.txt"]).contains("[0] app-stacks.txt"));
    assert_eq!(daemon.ok(&["unload", "0"]), "Unloaded graph 0 \"app-stacks.txt\". 0 graphs left.\n");
    assert_eq!(daemon.ok(&["list"]), "No graphs loaded.\n");
}

#[test]
fn load_formats_and_query() {
    let daemon = Daemon::start("query");
    daemon.write("app-stacks.txt", STACKS.as_bytes());
    daemon.write("app-stacks.txt.gz", &gzip(STACKS.as_bytes()));
    let jfr = jfr_fixture();

    // Relative paths resolve against the client's working directory (the test's HOME). Loaded raw
    // here and in most tests, so that node and function IDs follow the input.
    let loaded = daemon.ok(&["load", "--raw", "app-stacks.txt", "app-stacks.txt.gz", jfr.to_str().unwrap()]);
    assert_eq!(
        loaded,
        "Loaded 3 graphs:\n\
         nodes  samples\n\
         \x20   9       14  [0] app-stacks.txt [raw]\n\
         \x20   9       14  [1] app-stacks.txt.gz [raw]\n\
         \x20 637      319  [2] cpu-sample.jfr (cpu) [raw]\n"
    );

    assert_eq!(
        daemon.ok(&["node", "0", "3"]),
        "Node [3] in \"app-stacks.txt [raw]\", function [f3]:\n\
         total  self\n\
         \x20   5     5  [3] c\n\
         Parents, nearest first:\n\
         \x20   8     0  [2] b\n\
         \x20  10     0  [1] a\n\
         \x20  14     0  [0] root\n"
    );
    assert!(daemon.ok(&["node", "0", "3", "--parent-depth", "1"]).ends_with("[2] b\n2 more. Show all with --parent-depth -1.\n"));
    assert_eq!(
        daemon.ok(&["children", "0", "0", "--limit", "1"]),
        "Children of [0] root in \"app-stacks.txt [raw]\" (2):\n\
         total  self\n\
         \x20  10     0  [1] a\n\
         1 more. Show more with --limit or --offset.\n"
    );
    assert!(daemon.ok(&["children", "0", "0", "--offset", "1"]).ends_with("  4     0  [6] x\n"));
    assert_eq!(
        daemon.ok(&["subtree", "0", "1"]),
        "Subtree of [1] a in \"app-stacks.txt [raw]\":\n\
         total  self\n\
         \x20  10     0  [1] a\n\
         \x20   8     0  ├─ [2] b\n\
         \x20   5     5  │  ├─ [3] c\n\
         \x20   3     3  │  └─ [4] d\n\
         \x20   2     2  └─ [5] e\n"
    );
    assert!(daemon.ok(&["subtree", "2", "0"]).contains("Busy.fib"));
    assert_eq!(daemon.ok(&["raw-name", "0", "2"]), "b\n");
    assert_eq!(daemon.ok(&["raw-name", "0", "f3"]), "c\n");

    assert_eq!(
        daemon.ok(&["callers", "0", "f3"]),
        "Callers of [f3] c in \"app-stacks.txt [raw]\", callers below callees. A line's total is the samples of its call chain to the function.\n\
         total\n\
         \x20   9  [f3] c\n\
         \x20   9  └─ [f2] b\n\
         \x20   5     ├─ [f1] a\n\
         \x20   5     │  └─ [0] root\n\
         \x20   4     └─ [f6] x\n\
         \x20   4        └─ [0] root\n"
    );
    // Under a node, chains stop at it.
    assert!(daemon.ok(&["callers", "0", "f3", "--under", "1"]).ends_with("    5  [f3] c\n    5  └─ [f2] b\n    5     └─ [f1] a\n"));
    // Recursive calls count once: every sample through f, once each, by outermost caller.
    daemon.write("rec-stacks.txt", b"a;f;f;g 3\nb;f 2\nb;f;f 1\n");
    daemon.ok(&["load", "--raw", "rec-stacks.txt"]); // 3
    let recursive = daemon.ok(&["callers", "3", "f2"]);
    assert!(recursive.contains("Recursive calls count once.\n"), "{recursive}");
    assert!(recursive.ends_with("    6  [f2] f\n    3  ├─ [f1] a\n    3  │  └─ [0] root\n    3  └─ [f4] b\n    3     └─ [0] root\n"), "{recursive}");

    assert_eq!(
        daemon.ok(&["aggregate", "0", "f2"]),
        "Created graph [4] \"app-stacks.txt [raw] [merged: b]\" (3 nodes).\n"
    );
    assert_eq!(
        daemon.ok(&["icicle", "0", "1"]),
        "Created graph [5] \"app-stacks.txt [raw] [icicle: a]\" (6 nodes). Its root's children rank functions by self samples: `stax children 5 0`.\n"
    );

    // Loading merges frame-type variants of the same method; --raw keeps them.
    daemon.write("jit-stacks.txt", b"run_[0];work_[j] 2\nrun_[1];work_[j] 3\n");
    assert!(daemon.ok(&["load", "--raw", "jit-stacks.txt"]).ends_with("    5  [6] jit-stacks.txt [raw]\n"));
    assert!(daemon.ok(&["children", "6", "0"]).contains("(2):"));
    assert!(daemon.ok(&["load", "jit-stacks.txt"]).ends_with("    3        5  [7] jit-stacks.txt\n"));
    assert!(daemon.ok(&["children", "7", "0"]).contains("(1):"));

    // IDs are never reused.
    assert!(daemon.ok(&["unload", "5"]).contains("Unloaded graph 5"));
    assert!(daemon.ok(&["load", "app-stacks.txt"]).contains("[8] app-stacks.txt\n"));
}

#[test]
fn diff() {
    let daemon = Daemon::start("diff");
    daemon.write("a-stacks.txt", b"main;work;parse 50\nmain;work;solve 30\nmain;io 20\n");
    daemon.write("b-stacks.txt", b"main;work;parse 40\nmain;work;solve 70\nmain;io 20\nmain;gc 5\n");
    daemon.ok(&["load", "a-stacks.txt", "b-stacks.txt"]);
    assert_eq!(
        daemon.ok(&["diff", "0", "1"]),
        "Created graph [2] \"a-stacks.txt vs b-stacks.txt [diff: delta]\" (7 nodes). A is graph 0, B is graph 1.\n"
    );
    // Ranked and budgeted by |Δ|: the unchanged `io` has no weight and drops out.
    assert_eq!(
        daemon.ok(&["subtree", "2", "0"]),
        "Subtree of [0] root in \"a-stacks.txt vs b-stacks.txt [diff: delta]\":\n\
         \x20 A    B    Δ  self Δ\n\
         100  135  +35       0  [0] root\n\
         100  135  +35       0  └─ [1] main\n\
         \x2080  110  +30       0     ├─ [2] work\n\
         \x2030   70  +40     +40     │  ├─ [3] solve\n\
         \x2050   40  -10     -10     │  └─ [4] parse\n\
         \x20 0    5   +5      +5     └─ [6] gc\n"
    );
    assert!(daemon.ok(&["list"]).contains("100 → 135  [2] a-stacks.txt vs b-stacks.txt [diff: delta]"));
    assert!(daemon.ok(&["subtree", "2", "0", "--max-lines", "3"]).ends_with("\n2 lines have children not shown. Show more with --max-lines <n>.\n"));

    // In `faster`, a net-slower node still shows the speedups inside it.
    daemon.ok(&["diff", "0", "1", "--mode", "faster"]);
    assert!(daemon.ok(&["subtree", "3", "2"]).ends_with("80  110  +30       0  [2] work\n50   40  -10     -10  └─ [4] parse\n"));

    for command in ["aggregate 2 f1", "icicle 2 1", "callers 2 f1", "top 2", "search 2 x", "fn 2 f1", "calls 2 f1", "diff 2 0"] {
        let args: Vec<&str> = command.split(' ').collect();
        assert!(daemon.fail(&args).contains("is not supported for diff graphs"), "{command}");
    }
}

#[test]
fn functions() {
    let daemon = Daemon::start("functions");
    daemon.write("fn-stacks.txt", b"main;a;c 5\nmain;b;c 3\nmain;b 2\nc;c 1\n");
    daemon.ok(&["load", "--raw", "fn-stacks.txt"]);
    assert_eq!(
        daemon.ok(&["top", "0"]),
        "Functions in \"fn-stacks.txt [raw]\" by self samples (4):\n\
         self  total\n\
         \x20  9      9  [f3] c\n\
         \x20  2      5  [f4] b\n\
         \x20  0      5  [f2] a\n\
         \x20  0     10  [f1] main\n"
    );
    assert!(daemon.ok(&["top", "0", "--limit", "1"]).ends_with("3 more (2 self samples). Show more with --limit or --offset.\n"));
    // Under node 4 (main;b): b's call of c, and b's own samples.
    assert!(daemon.ok(&["top", "0", "--under", "4"]).ends_with("   3      3  [f3] c\n   2      5  [f4] b\n"));
    assert_eq!(
        daemon.ok(&["search", "0", "^[ab]$", "--limit", "1"]),
        "Functions matching /^[ab]$/i in \"fn-stacks.txt [raw]\" by total samples (2 of 4):\n\
         total  self\n\
         \x20   5     0  [f2] a\n\
         1 more. Show more with --limit or --offset.\n"
    );
    assert!(daemon.ok(&["search", "0", "^[ab]$", "--by", "self"]).contains("self  total\n   2      5  [f4] b\n"));
    assert_eq!(daemon.ok(&["search", "0", "zzz"]), "Functions matching /zzz/i in \"fn-stacks.txt [raw]\" by total samples (0 of 4): none.\n");

    assert_eq!(
        daemon.ok(&["fn", "0", "f3"]),
        "[f3] c in \"fn-stacks.txt [raw]\": total 9, self 9, 4 nodes\n\
         Callers, with the samples on each call:\n\
         5  [f2] a\n\
         3  [f4] b\n\
         1  (stack root)\n\
         1  [f3] c\n\
         Callees:\n\
         1  [f3] c\n\
         Heaviest nodes:\n\
         5  [3]\n\
         3  [5]\n\
         1  [6]\n\
         1  [7]\n"
    );
    // Under node 4 (main;b): only b's call of c.
    assert_eq!(
        daemon.ok(&["fn", "0", "f3", "--under", "4"]),
        "[f3] c in \"fn-stacks.txt [raw]\" under [4] b: total 3, self 3, 1 node\n\
         Callers, with the samples on each call:\n\
         3  [f4] b\n\
         Callees: none\n\
         Heaviest nodes:\n\
         3  [5]\n"
    );
    assert!(daemon.ok(&["fn", "0", "f4", "--under", "4"]).contains("Callers, with the samples on each call:\n5  (caller above [4])\n"));
    assert!(daemon.fail(&["fn", "0", "f2", "--under", "4"]).contains("[f2] a has no samples in \"fn-stacks.txt [raw]\" under [4] b."));
    let limited = daemon.ok(&["fn", "0", "3", "--limit", "1"]);
    assert!(limited.contains("5  [f2] a\n3 more (5 samples). Show more with --limit.\n"), "{limited}");
    assert!(limited.ends_with("5  [3]\n3 more. Show more with --limit.\n"), "{limited}");
    assert!(daemon.ok(&["fn", "0", "f1"]).contains("Callers, with the samples on each call:\n10  (stack root)\n"));

    // In a merged graph the root is the merged function, and only samples under it count. Functions
    // keep their IDs from the graph it was made from.
    daemon.write("scoped-stacks.txt", b"a;f;x 3\nb;f;g;x 2\nc;x 1\n");
    daemon.ok(&["load", "--raw", "scoped-stacks.txt"]); // 1
    daemon.ok(&["aggregate", "1", "f2"]); // 2
    assert_eq!(
        daemon.ok(&["fn", "2", "f2"]),
        "[f2] f in \"scoped-stacks.txt [raw] [merged: f]\": total 5, self 0, 1 node\n\
         Callers, with the samples on each call:\n\
         5  (stack root)\n\
         Callees:\n\
         3  [f3] x\n\
         2  [f5] g\n\
         Heaviest nodes:\n\
         5  [0]\n"
    );
    assert!(daemon.ok(&["top", "2"]).contains("   5      5  [f3] x\n"));
    daemon.ok(&["icicle", "1", "0"]); // 3
    assert!(daemon.fail(&["top", "3"]).contains("not supported for icicle graphs"));

    assert!(daemon.fail(&["fn", "0", "f0"]).contains("No function f0"));
    assert!(daemon.fail(&["fn", "0", "f99"]).contains("No function f99"));
    assert_eq!(daemon.run(&["fn", "0", "fx"]).status.code(), Some(2));
}

#[test]
fn calls() {
    let daemon = Daemon::start("calls");
    // g is called three ways under f, and from `other`, whose samples are outside f's scope.
    daemon.write("calls-stacks.txt", b"main;f;a;g;h 6\nmain;f;b;g;k 2\nmain;f;g 1\nmain;f;f 1\nmain;other;g;z 50\n");
    daemon.ok(&["load", "--raw", "calls-stacks.txt"]);
    // g expands under its heaviest call; the root's own recursion (f;f) is folded into it, as in
    // `aggregate`. A (*) line's self is on the line where it is expanded.
    assert_eq!(
        daemon.ok(&["calls", "0", "f2"]),
        "Calls under [f2] f in \"calls-stacks.txt [raw]\", merged across its call sites. (*): its callees are shown at another line.\n\
         total  self\n\
         \x20  10     1  [f2] f\n\
         \x20   6     0  ├─ [f3] a\n\
         \x20   6     1  │  └─ [f4] g  (in all calls: 9)\n\
         \x20   6     6  │     ├─ [f5] h\n\
         \x20   2     2  │     └─ [f7] k\n\
         \x20   2     0  ├─ [f6] b\n\
         \x20   2        │  └─ [f4] g (*)\n\
         \x20   1        └─ [f4] g (*)\n"
    );
    // A function calling itself is a (*) line under its own.
    daemon.write("recursive-stacks.txt", b"main;f;g;g;x 4\nmain;f;g 1\n");
    daemon.ok(&["load", "--raw", "recursive-stacks.txt"]); // 1
    let recursive = daemon.ok(&["calls", "1", "2"]);
    assert!(recursive.contains("    5     1  └─ [f3] g\n"), "{recursive}");
    assert!(recursive.contains("    4           ├─ [f3] g (*)\n"), "{recursive}");
    // A tree the line budget cuts says so.
    assert!(daemon.ok(&["calls", "0", "f2", "--max-lines", "2"]).ends_with(
        "    6     0  └─ [f3] a\n2 lines have callees not shown. Show more with --max-lines <n>.\n"
    ));
    assert!(!daemon.ok(&["calls", "0", "f2"]).contains("not shown"));

    // Under `other` (node 11), g's only call site is there, with its own callee.
    let under = daemon.ok(&["calls", "0", "f4", "--under", "11"]);
    assert!(under.ends_with("   50     0  [f4] g\n   50    50  └─ [f9] z\n"), "{under}");
    assert!(under.contains(" under [11] other,"), "{under}");
    assert!(daemon.fail(&["calls", "0", "f5", "--under", "11"]).contains("has no samples"));

    assert!(daemon.fail(&["calls", "0", "f0"]).contains("No function f0"));
    daemon.ok(&["icicle", "0", "0"]); // 2
    assert!(daemon.fail(&["calls", "2", "f1"]).contains("not supported for icicle graphs"));
}

/// Two daemons, each a separate process, derive the same graphs with the same node ids.
#[test]
fn derived_graphs_are_reproducible() {
    // Many call sites of f, with recursion, so any hash-ordered walk would reorder nodes.
    let stacks: String = (0..300)
        .map(|i| format!("main;caller{i};f;g{};f;leaf{} {}\n", i % 7, i % 11, i % 5 + 1))
        .collect();
    let dumps: Vec<String> = ["repro-a", "repro-b"]
        .iter()
        .map(|name| {
            let daemon = Daemon::start(name);
            daemon.write("app-stacks.txt", stacks.as_bytes());
            daemon.ok(&["load", "app-stacks.txt"]);
            let f = daemon.ok(&["search", "0", "^f$"]);
            assert!(f.contains("[f3] f"), "{f}");
            daemon.ok(&["aggregate", "0", "f3"]); // 1
            daemon.ok(&["icicle", "0", "0"]); // 2
            daemon.ok(&["diff", "0", "1"]); // 3
            (0..4)
                .map(|g| daemon.ok(&["subtree", &g.to_string(), "0", "--no-collapse", "--max-lines", "100000"]))
                .collect()
        })
        .collect();
    assert!(dumps[0].contains("[1] "), "dumps should list nodes: {}", dumps[0]);
    assert_eq!(dumps[0], dumps[1]);
}

#[test]
fn errors() {
    let daemon = Daemon::start("errors");
    daemon.write("app-stacks.txt", STACKS.as_bytes());
    daemon.ok(&["load", "app-stacks.txt"]);
    daemon.ok(&["unload", "0"]);
    assert_eq!(daemon.fail(&["top", "0"]), "No graphs loaded. Load a profile with `stax load <file>...`.\n");
    daemon.ok(&["load", "app-stacks.txt"]); // 1

    assert!(daemon.fail(&["top", "7"]).contains("Graph 7 not found. Loaded: 1."));
    assert!(daemon.fail(&["node", "1", "99"]).contains("out of range"));
    assert!(daemon.fail(&["search", "1", "("]).contains("Invalid pattern"));
    assert!(daemon.fail(&["callers", "1", "f99"]).contains("No function f99"));
    assert!(daemon.fail(&["top", "1", "--under", "99"]).contains("out of range"));
    assert!(daemon.fail(&["load", "missing.txt"]).contains("missing.txt"));

    // A file that fails to parse loads nothing, including the files before it.
    daemon.write("bad-stacks.txt", b"a;b 1\nnot a stack\n");
    assert!(daemon.fail(&["load", "app-stacks.txt", "bad-stacks.txt"]).contains("bad-stacks.txt: line 2: malformed input"));
    daemon.write("empty-stacks.txt", b"");
    assert!(daemon.fail(&["load", "empty-stacks.txt"]).contains("empty-stacks.txt: no samples."));
    assert!(daemon.ok(&["list"]).starts_with("1 graph loaded"));

    // Usage errors are reported by the client, with clap's exit code.
    assert_eq!(daemon.run(&["aggregate", "1"]).status.code(), Some(2));
    assert_eq!(daemon.run(&["hot-path", "1", "0"]).status.code(), Some(2));
}
