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
stax serve                            # start the daemon
stax load app-stacks.txt rec.jfr      # -stacks.txt, -stacks.txt.gz, or .jfr
stax list                             # graph IDs
stax search 0 'traverseGraph' --limit 5
stax subtree 0 0                      # node 0 is the root
stax simplify 0                       # merge JIT/inlined/interpreted variants of each frame
stax shutdown
```

Run `stax --help` and `stax <command> --help` for every command. Patterns are case-insensitive
regular expressions matched against full (raw) frame names, which can differ from the names shown;
`stax raw-name` prints one. Errors go to stderr with exit code 1.

The daemon listens on a Unix socket in `$XDG_RUNTIME_DIR/stax/`, or `~/.stax/` when that is unset,
and logs to `~/.stax/daemon.log`. Windows is not supported. The CLI is a Cargo workspace member at
`src/main/rust/cli`, sharing the graph code with the WASM build.

## Managed Toolchains

This project manages its own versions of Node and Rust to ensure build reproducibility and avoid
developers needing to install these tools themselves.

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
