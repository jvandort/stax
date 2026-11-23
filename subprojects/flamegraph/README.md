# Flamegraph

An interactive flamegraph visualizer for async-profiler and JVM stack traces. Stacks are rendered
in the browser with no server — the entire application ships as a single self-contained `index.html` file.

This application supports a number of features:
- Embedding `-stacks.txt` and `.jfr` files directly into the `index.html` file
- Loading `-stacks.txt` and `.jfr` files at runtime
- Displaying multiple stacks files
- Icicle graph generation
- Merging stack entries with the same name
- Zoom, navigation
- Search

## Building the production version

Run `./gradlew :stax:build`.

## Application Architecture

The main application is written in React and TypeScript. The entry-point is `src/main/ts/App.tsx`.
Gradle delegates to the `Vite` build system for building the web application.

The application relies on web workers to perform heavy-lifting. Much of the performance-sensitive
data processing logic is implemented in Rust and compiled to a WASM binary. This binary is injected
into the web application and loaded at runtime. The Rust entry-point is `src/main/rust/src/lib.rs`.

The final web application is packaged as a single file, `index.html`, in `build/vite`. This file
supports embedding DEFLATE-compressed, Base64-encoded profile files — `-stacks.txt` sample files
or `.jfr` recordings. On startup, the embedded files are decoded and decompressed before
further processing. Entries named `*.jfr` are decoded as JFR recordings (producing one graph per
sample kind present, e.g. cpu and wall). Everything else is parsed as `stacks.txt` files. The
application also supports loading additional stacks or JFR files at runtime using file pickers.

The resulting `index.html` is embedded as a resource in a small Java wrapper application, which
accepts a list of profile files as input, compresses and encodes them, and injects them into an
output `index.html`. The Java entry-point is `FlamegraphGenerator.java` in `src/main/java/`.
Generate a demo with e.g.
`./gradlew :stax:buildDemo --embed "/abs/path/profile-stacks.txt /abs/path/recording.jfr"`.

## Command-line interface

`stax` queries profiles from the command line, for scripts and coding agents. A background daemon
holds the loaded graphs, so each command is a separate, short-lived process.

```
./gradlew :stax:buildCli              # builds build/cargo/<host target>/cli/stax
stax                                  # the guide: every command and option
stax load app-stacks.txt rec.jfr      # starts the daemon; prints graph IDs (0 and 1)
stax top 0                            # functions by self samples
stax search 0 traverseGraph           # find a function's ID by name
stax fn 0 f12                         # one function: callers, callees, heaviest nodes
stax calls 0 f12                      # what f12 calls, as a tree
stax callers 0 f12                    # every call chain that reaches f12
stax subtree 0 0                      # node 0 is the root
stax aggregate 0 f12                  # graph 2: f12's call sites merged; f12 is still f12
stax diff 0 1 --mode slower           # compare graph 0 (A) with graph 1 (B)
stax unload 2
stax shutdown
```

`stax` with no arguments prints a guide to every command and option, so that an agent that has never
seen the tool learns it in one call. Patterns are case-insensitive regular expressions matched against
full (raw) frame names, which can differ from the names shown. `stax raw-name` prints one. Errors go to
stderr with exit code 1.

`stax diff` works like the flamegraph's diff view. Lines show A, B and Δ = B − A, and the tree
commands rank and budget them by the mode: `delta` (|Δ|), `slower` or `faster`. The other commands
don't support diff graphs.

The daemon listens on a Unix socket in `$XDG_RUNTIME_DIR/stax/`, or `~/.stax/` when that is unset,
and logs to `~/.stax/daemon.log`. Windows is not supported. The CLI is a Cargo workspace member at
`src/main/rust/cli`, sharing the graph code with the WASM build.

## Managed Toolchains

This project manages its own versions of Node and Rust to ensure build reproducibility and avoid
developers needing to install these tools themselves. The one exception is a C linker: Rust links its
build scripts and proc-macros with the system's `cc`, so install `build-essential` (Debian/Ubuntu) or the
Xcode Command Line Tools (macOS). The Rust tasks fail early with that message if `cc` is missing.

### Rust & Cargo

You can run arbitrary `cargo` commands using the managed installation.

* **Unit tests:** `./gradlew :stax:cargoTest`
* **Lint:** `./gradlew :stax:cargo --cmd "clippy"`
* **Format:** `./gradlew :stax:cargo --cmd "fmt"`
* **Update crates:** `./gradlew :stax:cargo --cmd "update"`

`Cargo.lock` is committed and the production build enforces it with `--locked`. After running
`update`, verify the changes and commit the updated `Cargo.lock` before building.

### Node & NPM

* **Type check:** `./gradlew :stax:typecheck`
* **NPM commands:** `./gradlew :stax:npm --cmd "install --save some-pkg"`
* **NPX commands:** `./gradlew :stax:npx --cmd "vitest run src/test/ts/diffRendering.test.ts"`

## Development

Development is facilitated with the Vite development server, which watches for changes to
TypeScript and Rust source files and performs hot-reloading on demand.

Start the dev server:

Run `./gradlew :stax:serveVite`

This serves a live development version of the app at http://localhost:5173/

### Running tests

**Rust:**
```
./gradlew :stax:cargoTest
```

**TypeScript (vitest, in `src/test/ts`):**
```
./gradlew :stax:tsTest
```

Both test suites also run automatically as part of `./gradlew :stax:check`.

**TypeScript (type check):**
```
./gradlew :stax:typecheck
```
