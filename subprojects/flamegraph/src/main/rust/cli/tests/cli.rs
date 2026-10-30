// End-to-end tests: drive the real binary against a daemon in an isolated home directory.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const STACKS: &str = "a;b;c 5\na;b;d 3\na;e 2\nx;b;c 4\n";

/// A daemon in its own `HOME`, stopped on drop.
struct Daemon {
    home: PathBuf,
}

impl Daemon {
    fn start(name: &str) -> Daemon {
        let home = std::env::temp_dir().join(format!("stax-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let daemon = Daemon { home };
        let out = daemon.run(&["serve"]);
        assert_eq!(out.status.code(), Some(0), "serve failed: {}", stderr(&out));
        daemon
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-data/cpu-sample.jfr")
}

#[test]
fn lifecycle() {
    let daemon = Daemon::start("lifecycle");
    assert!(daemon.ok(&["serve"]).contains("already running"));
    assert_eq!(daemon.ok(&["list"]), "No profiles loaded.\n");
    assert!(daemon.ok(&["shutdown"]).contains("stopped"));
    assert!(daemon.fail(&["list"]).contains("no stax daemon running"));
    assert!(daemon.fail(&["shutdown"]).contains("no stax daemon running"));
    // A stale socket from the stopped daemon doesn't block a new one.
    assert!(daemon.ok(&["serve"]).contains("started"));
}

#[test]
fn load_formats_and_query() {
    let daemon = Daemon::start("query");
    daemon.write("app-stacks.txt", STACKS.as_bytes());
    daemon.write("app-stacks.txt.gz", &gzip(STACKS.as_bytes()));
    let jfr = jfr_fixture();

    // Relative paths resolve against the client's working directory (the test's HOME).
    let loaded = daemon.ok(&["load", "app-stacks.txt", "app-stacks.txt.gz", jfr.to_str().unwrap()]);
    assert_eq!(
        loaded,
        "Loaded 3 graph(s):\n\
         \x20 [0] app-stacks.txt — 9 nodes, 14 total samples, root node: 0\n\
         \x20 [1] app-stacks.txt.gz — 9 nodes, 14 total samples, root node: 0\n\
         \x20 [2] cpu-sample.jfr (cpu) — 636 nodes, 319 total samples, root node: 0\n"
    );

    assert_eq!(
        daemon.ok(&["search", "0", r"^b$"]),
        "2 node(s) matching /^b$/i in \"app-stacks.txt\" — combined: 12 samples (85.7%), nested matches counted once (showing 2 from offset 0):\n\
         \x20 [2] b, 8 samples (57.1%), parent: [1] a\n\
         \x20 [7] b, 4 samples (28.6%), parent: [6] x\n"
    );
    // No matches is a successful search.
    assert!(daemon.ok(&["search", "0", "zzz"]).starts_with("No nodes matching"));

    assert!(daemon.ok(&["node", "0", "3"]).contains("  [2] b\n  [1] a\n  [0] root"));
    assert!(daemon.ok(&["children", "0", "0", "--limit", "1"]).contains("(1 more not shown)"));
    assert!(daemon.ok(&["hot-path", "2", "0"]).contains("Busy.fib"));
    assert!(daemon.ok(&["subtree", "0", "0"]).starts_with("Subtree summary of [0]"));
    assert!(daemon.ok(&["callers", "0", "--pattern", "^c$"]).contains("callers: 2"));

    // Recursive calls count once: every sample through f, once each, by outermost caller.
    daemon.write("rec-stacks.txt", b"a;f;f;g 3\nb;f 2\nb;f;f 1\n");
    daemon.ok(&["load", "rec-stacks.txt"]);
    assert_eq!(
        daemon.ok(&["callers", "3", "--pattern", "^f$"]),
        "Callers of \"f\" in \"rec-stacks.txt\" (4 instances, nested recursive calls counted once, 5 lines):\n\
         f, 6 samples (100.0%), callers: 2\n\
         ├─ a, 3 samples (50.0%), callers: 1\n\
         │  └─ root, 3 samples (50.0%), self: 3 samples (50.0%)\n\
         └─ b, 3 samples (50.0%), callers: 1\n\
         \x20  └─ root, 3 samples (50.0%), self: 3 samples (50.0%)\n"
    );
    assert_eq!(daemon.ok(&["raw-name", "0", "2"]), "b\n");

    assert!(daemon.ok(&["aggregate", "0", "--pattern", "^b$"]).starts_with("Created merged graph [4]"));
    assert!(daemon.ok(&["icicle", "0", "--node", "0"]).starts_with("Created icicle graph [5]"));

    // Frame-type variants of the same method merge.
    daemon.write("jit-stacks.txt", b"run_[0];work_[j] 2\nrun_[1];work_[j] 3\n");
    daemon.ok(&["load", "jit-stacks.txt"]);
    assert!(daemon.ok(&["children", "6", "0"]).contains("(2/2)"));
    assert_eq!(
        daemon.ok(&["simplify", "6"]),
        "Created simplified graph [7] \"jit-stacks.txt [simplified]\" with 3 nodes, root node: 0.\n"
    );
    assert!(daemon.ok(&["children", "7", "0"]).contains("(1/1)"));

    // IDs are never reused.
    assert!(daemon.ok(&["unload", "5"]).contains("Unloaded graph 5"));
    assert!(daemon.ok(&["load", "app-stacks.txt"]).contains("[8] app-stacks.txt"));
}

#[test]
fn errors() {
    let daemon = Daemon::start("errors");
    assert_eq!(daemon.fail(&["search", "0", "x"]), "No profiles loaded.\n");
    daemon.write("app-stacks.txt", STACKS.as_bytes());
    daemon.ok(&["load", "app-stacks.txt"]);

    assert!(daemon.fail(&["search", "7", "x"]).contains("Graph 7 not found. Available IDs: 0"));
    assert!(daemon.fail(&["node", "0", "99"]).contains("out of range"));
    assert!(daemon.fail(&["search", "0", "("]).contains("Invalid regex"));
    assert!(daemon.fail(&["callers", "0", "--pattern", "zzz"]).contains("No nodes matching"));
    assert!(daemon.fail(&["load", "missing.txt"]).contains("missing.txt"));

    // A file that fails to parse loads nothing, including the files before it.
    daemon.write("bad-stacks.txt", b"not a stack\n");
    assert!(daemon.fail(&["load", "app-stacks.txt", "bad-stacks.txt"]).contains("malformed input"));
    assert!(daemon.ok(&["list"]).starts_with("1 profile(s) loaded"));

    // Usage errors are reported by the client, with clap's exit code.
    assert_eq!(daemon.run(&["aggregate", "0"]).status.code(), Some(2));
}
