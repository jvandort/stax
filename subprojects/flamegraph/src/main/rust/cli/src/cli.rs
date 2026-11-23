use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::graph::DiffMode;

/// A function's ID, as printed by `top`, `fn` and `node`: `f12` or `12`.
#[derive(Clone, Copy, Debug)]
pub struct FunctionId(pub usize);

fn parse_function_id(s: &str) -> Result<FunctionId, String> {
    s.strip_prefix(['f', 'F'])
        .unwrap_or(s)
        .parse()
        .map(FunctionId)
        .map_err(|_| format!("expected a function ID such as f12, got {s:?}"))
}

/// A node ID (`123`) or a function ID (`f12`).
#[derive(Clone, Copy, Debug)]
pub enum NodeOrFunction {
    Node(usize),
    Function(usize),
}

fn parse_node_or_function(s: &str) -> Result<NodeOrFunction, String> {
    if s.starts_with(['f', 'F']) {
        parse_function_id(s).map(|f| NodeOrFunction::Function(f.0))
    } else {
        s.parse().map(NodeOrFunction::Node).map_err(|_| format!("expected a node ID such as 123 or a function ID such as f12, got {s:?}"))
    }
}

/// What `top` and `search` rank functions by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum RankBy {
    /// Samples in which the function is running, not one of its callees.
    #[value(name = "self")]
    SelfSamples,
    /// Samples in which the function is anywhere on the stack.
    Total,
}

/// The user guide: what `stax`, `stax -h`, `stax --help` and `stax help` print. Every command and
/// option, so that one call tells an agent everything. A test checks that it names every command
/// and long option the parser has.
pub const GUIDE: &str = "\
stax: explore profiles from the shell.

Loads Java Flight Recorder recordings (.jfr) and collapsed stacks (one stack per line, frames
separated by ';', followed by its sample count; .gz is unzipped). A daemon holds them in memory,
so queries are instant even on gigabyte profiles. A graph stays in memory until you unload it:
managing memory is up to you.

Loading simplifies a profile: frames of the same function that the profile tells apart by
runtime details or metadata (on the JVM: interpreted, compiled and inlined frames, line numbers,
generated lambda and proxy names) are merged, which removes details that could detract and
distract from the real code flow. `load --raw` keeps them.

Numbers are sample counts. total: samples in a frame and everything it calls. self: samples in
the frame alone. A node ([123]) is one frame on one call path; node 0 is the graph's root. A
function ([f12]) is every node with the same name, across all call paths. Patterns are
case-insensitive regular expressions.

Daemon
  stax load <file>... [--raw]
      Loads profiles and prints each new graph's ID. Starts the daemon if it isn't running.
  stax list
      Loaded graphs: ID, name, node count, samples.
  stax unload <graph>
      Frees a graph. Unload every graph you no longer need: nothing else frees them. IDs aren't
      reused.
  stax shutdown
      Stops the daemon and frees every graph.

Functions
  stax top <graph> [--by self|total] [--under <node>] [--limit <n>] [--offset <n>]
      Functions ranked by self samples (default) or total: where the time goes.
  stax search <graph> <pattern> [--by self|total] [--under <node>] [--limit <n>] [--offset <n>]
      Functions whose names match <pattern>, ranked by total (default) or self samples: how you
      find a function's ID.
  stax fn <graph> <fN> [--under <node>] [--limit <n>]
      One function: its callers and callees with the samples on each call, and its heaviest
      nodes.
  stax calls <graph> <fN> [--under <node>] [--max-lines <n>] [--no-collapse]
      Everything the function calls, as a tree, merged across its call sites. A function called
      from several places is expanded once; its other calls are marked (*).
  stax callers <graph> <fN> [--under <node>] [--max-lines <n>] [--no-collapse]
      Every call chain that reaches the function, as a tree with callers below.

Call tree
  stax subtree <graph> <node> [--max-lines <n>] [--no-collapse]
      The tree under a node; heavier branches get more of the lines.
  stax children <graph> <node> [--limit <n>] [--offset <n>]
      A node's children.
  stax node <graph> <node> [--parent-depth <n>]
      A node's samples, its function, and its parents (10 levels by default; -1 for all).
  stax raw-name <graph> <node|fN>
      The full name, when the shown name is abbreviated.

Derived graphs (each prints the new graph's ID)
  stax aggregate <graph> <fN>
      One tree rooted at the function, with all its call sites merged.
  stax icicle <graph> <node>
      The subtree under a node, inverted to rank self time: the root's children are the
      functions the samples are spent in, heaviest first, and below each are its callers.
  stax diff <a> <b> [--mode delta|slower|faster]
      Compares two graphs. Lines show A, B and Δ = B - A, ranked by |Δ| (delta), growth (slower)
      or shrinkage (faster). Raw input graphs are simplified first. Call tree commands work on
      the result; the others don't.

Options
  --limit <n>       Entries to list (default 10; -1 for all). --offset <n> skips the first n.
  --max-lines <n>   Lines a tree may use (default 40).
  --no-collapse     Trees fold a run of frames that pass all their samples straight through into
                    one \"... N frames ...\" line; this shows every frame.
  --under <node>    Counts only the samples under that node.
";

/// Explore profiles from the shell. Run `stax` with no arguments for the guide.
///
/// Both the client and the daemon parse arguments with this definition: the client to report usage
/// errors without a daemon, the daemon to run the command.
#[derive(Parser, Debug)]
#[command(name = "stax", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// A list's paging options.
#[derive(Args, Debug, Clone, Copy)]
pub struct Page {
    /// Entries to list; -1 for all.
    #[arg(long, default_value_t = 10, allow_negative_numbers = true,
          value_parser = clap::value_parser!(i64).range(-1..))]
    pub limit: i64,
    /// Entries to skip.
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
}

/// A tree's line budget.
#[derive(Args, Debug, Clone, Copy)]
pub struct TreeOptions {
    /// Lines the tree may use.
    #[arg(long, default_value_t = 40)]
    pub max_lines: usize,
    /// Show every frame of a pass-through run instead of folding it into one line.
    #[arg(long)]
    pub no_collapse: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Stop the daemon and free every graph.
    Shutdown,

    /// Load profiles (.jfr recordings, or collapsed stacks, gzipped if .gz), simplified, and print
    /// each new graph's ID. Starts the daemon if it isn't running. Either every file loads, or none
    /// does.
    Load {
        /// Profile files.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Keep the frames that loading would merge: interpreted, compiled and inlined frames, line
        /// numbers, generated lambda and proxy names.
        #[arg(long)]
        raw: bool,
    },

    /// Free a graph. Unload every graph you no longer need: nothing else frees them. IDs aren't
    /// reused.
    Unload {
        /// Graph ID, from `stax list`.
        graph: u32,
    },

    /// List loaded graphs: ID, name, node count, samples.
    List,

    /// Rank functions by self samples (default) or total: where the time goes.
    Top {
        /// Graph ID, from `stax list`.
        graph: u32,
        /// What to rank by.
        #[arg(long, value_enum, default_value_t = RankBy::SelfSamples)]
        by: RankBy,
        /// Count only the samples under this node.
        #[arg(long, value_name = "NODE", default_value_t = 0)]
        under: usize,
        #[command(flatten)]
        page: Page,
    },

    /// Find functions whose names match a pattern, ranked by total (default) or self samples.
    Search {
        /// Graph ID, from `stax list`.
        graph: u32,
        /// Case-insensitive regular expression, matched against full names.
        pattern: String,
        /// What to rank by.
        #[arg(long, value_enum, default_value_t = RankBy::Total)]
        by: RankBy,
        /// Count only the samples under this node.
        #[arg(long, value_name = "NODE", default_value_t = 0)]
        under: usize,
        #[command(flatten)]
        page: Page,
    },

    /// Show one function: its callers and callees with the samples on each call, and its heaviest
    /// nodes.
    Fn {
        /// Graph ID, from `stax list`.
        graph: u32,
        /// Function ID, e.g. f12.
        #[arg(value_parser = parse_function_id)]
        function: FunctionId,
        /// Count only the samples under this node.
        #[arg(long, value_name = "NODE", default_value_t = 0)]
        under: usize,
        /// Entries in each list; -1 for all.
        #[arg(long, default_value_t = 10, allow_negative_numbers = true,
              value_parser = clap::value_parser!(i64).range(-1..))]
        limit: i64,
    },

    /// Draw everything a function calls as a tree, merged across its call sites. A function called
    /// from several places is expanded once; its other calls are marked (*).
    Calls {
        /// Graph ID, from `stax list`.
        graph: u32,
        /// Function ID, e.g. f12.
        #[arg(value_parser = parse_function_id)]
        function: FunctionId,
        /// Merge only the function's call sites under this node.
        #[arg(long, value_name = "NODE", default_value_t = 0)]
        under: usize,
        #[command(flatten)]
        tree: TreeOptions,
    },

    /// Draw every call chain that reaches a function, as a tree with callers below.
    Callers {
        /// Graph ID, from `stax list`.
        graph: u32,
        /// Function ID, e.g. f12.
        #[arg(value_parser = parse_function_id)]
        function: FunctionId,
        /// Count only the samples under this node; chains stop there.
        #[arg(long, value_name = "NODE", default_value_t = 0)]
        under: usize,
        #[command(flatten)]
        tree: TreeOptions,
    },

    /// Draw the tree under a node; heavier branches get more of the lines.
    Subtree {
        /// Graph ID, from `stax list`.
        graph: u32,
        /// Node ID; 0 is the root.
        node: usize,
        #[command(flatten)]
        tree: TreeOptions,
    },

    /// List a node's children.
    Children {
        /// Graph ID, from `stax list`.
        graph: u32,
        /// Node ID; 0 is the root.
        node: usize,
        #[command(flatten)]
        page: Page,
    },

    /// Show a node's samples, its function, and its parents.
    Node {
        /// Graph ID, from `stax list`.
        graph: u32,
        /// Node ID; 0 is the root.
        node: usize,
        /// Parents to show; -1 for all.
        #[arg(long, default_value_t = 10, allow_negative_numbers = true,
              value_parser = clap::value_parser!(i64).range(-1..))]
        parent_depth: i64,
    },

    /// Print a node's or function's full name, when the shown name is abbreviated.
    RawName {
        /// Graph ID, from `stax list`.
        graph: u32,
        /// Node ID (123) or function ID (f12).
        #[arg(value_parser = parse_node_or_function)]
        id: NodeOrFunction,
    },

    /// Create a graph rooted at a function, with all its call sites merged.
    Aggregate {
        /// Graph ID, from `stax list`.
        graph: u32,
        /// Function ID, e.g. f12.
        #[arg(value_parser = parse_function_id)]
        function: FunctionId,
    },

    /// Create an inverted graph of the subtree under a node, to rank self time: the root's children
    /// are the functions the samples are spent in, and below each are its callers.
    Icicle {
        /// Graph ID, from `stax list`.
        graph: u32,
        /// Node ID; 0 is the root.
        node: usize,
    },

    /// Create a graph comparing two graphs: lines show A, B and Δ = B - A. Raw input graphs
    /// are simplified first. Call tree commands work on the result; the others don't.
    Diff {
        /// Graph A's ID.
        a: u32,
        /// Graph B's ID.
        b: u32,
        /// What to rank by: delta = |Δ|, slower = Δ > 0 only, faster = Δ < 0 only.
        #[arg(long, value_enum, default_value_t = DiffMode::Delta)]
        mode: DiffMode,
    },

    /// Run the daemon in the foreground. Started by `load`.
    #[command(name = "__daemon", hide = true)]
    Daemon,
}
