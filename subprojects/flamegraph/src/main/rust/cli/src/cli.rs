use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Query profiles held in memory by a background stax daemon.
///
/// Start the daemon with `stax serve`, load profiles with `stax load`, then query them by graph ID
/// (from `stax list`) and node ID (from the output of other commands). Node 0 is a graph's root.
/// Patterns are case-insensitive regular expressions matched against full frame names.
///
/// Both the client and the daemon parse arguments with this definition: the client to report usage
/// errors without a daemon, the daemon to run the command.
#[derive(Parser, Debug)]
#[command(name = "stax", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the daemon in the background. Does nothing if one is already running.
    Serve,

    /// Stop the daemon, discarding all loaded graphs.
    Shutdown,

    /// Load profiles: `-stacks.txt`, `-stacks.txt.gz`, or `.jfr` recordings (one graph per sample
    /// kind present, e.g. cpu and wall). Either every file loads, or none does.
    Load {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },

    /// Unload a graph to free memory. Graph IDs are never reused.
    Unload { graph: u32 },

    /// List loaded graphs with their IDs, node counts, and total sample counts.
    List,

    /// Search for nodes whose names match a pattern. Results are sorted by sample count
    /// descending, with total and self samples. Use `node` to see a result's ancestor chain.
    Search {
        graph: u32,
        pattern: String,
        /// Maximum number of results.
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Number of results to skip, for pagination.
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Restrict the search to the subtree rooted at this node.
        #[arg(long, value_name = "NODE", default_value_t = 0)]
        under: usize,
    },

    /// Summarize a node: name, total and self samples, child count, and parent chain to the root.
    Node {
        graph: u32,
        node: usize,
        /// Levels of parent chain to show; -1 for all.
        #[arg(long, default_value_t = 10, allow_negative_numbers = true,
              value_parser = clap::value_parser!(i64).range(-1..))]
        parent_depth: i64,
    },

    /// List a node's children, sorted by sample count descending. Percentages are relative to
    /// the node.
    Children {
        graph: u32,
        node: usize,
        /// Maximum number of children to show; -1 for all.
        #[arg(long, default_value_t = 10, allow_negative_numbers = true,
              value_parser = clap::value_parser!(i64).range(-1..))]
        limit: i64,
    },

    /// Merge all call sites of a function into a new graph rooted at that function.
    Aggregate {
        graph: u32,
        #[command(flatten)]
        target: Target,
    },

    /// Create an inverted (icicle) graph of the subtree under a node: leaves become roots.
    Icicle {
        graph: u32,
        #[command(flatten)]
        target: Target,
    },

    /// Print a node's full raw name, for when the display name is abbreviated.
    RawName { graph: u32, node: usize },

    /// Follow the highest-sample child at each level from a node down to the hottest leaf.
    HotPath {
        graph: u32,
        node: usize,
        /// Maximum depth to follow.
        #[arg(long)]
        depth_limit: Option<usize>,
        /// Maximum number of output lines.
        #[arg(long, default_value_t = 40)]
        max_lines: usize,
        /// Show consecutive frames with identical totals instead of collapsing them.
        #[arg(long)]
        no_collapse: bool,
    },

    /// Summarize the subtree under a node, giving heavier branches more of the line budget.
    Subtree {
        graph: u32,
        node: usize,
        /// Maximum number of output lines.
        #[arg(long, default_value_t = 40)]
        max_lines: usize,
        /// Show pass-through chains with identical totals instead of collapsing them.
        #[arg(long)]
        no_collapse: bool,
    },

    /// Show who calls a function, aggregated across all its call sites: `subtree` in reverse.
    Callers {
        graph: u32,
        #[command(flatten)]
        target: Target,
        /// Maximum number of output lines.
        #[arg(long, default_value_t = 40)]
        max_lines: usize,
    },

    /// Run the daemon in the foreground. Started by `serve`.
    #[command(name = "__daemon", hide = true)]
    Daemon,
}

/// The node a command operates on: given directly, or as the highest-sample node whose name
/// matches a pattern.
#[derive(Args, Debug)]
#[group(required = true, multiple = false)]
pub struct Target {
    /// Node ID.
    #[arg(long)]
    pub node: Option<usize>,
    /// Pattern; the highest-sample matching node is used.
    #[arg(long)]
    pub pattern: Option<String>,
}
