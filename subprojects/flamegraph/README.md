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

Run `./gradlew :flamegraph:build`.

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
`./gradlew :flamegraph:buildDemo --stacks "/abs/path/profile-stacks.txt /abs/path/recording.jfr"`.

## Managed Toolchains

This project manages its own versions of Node and Rust to ensure build reproducibility and avoid
developers needing to install these tools themselves.

### Rust & Cargo

You can run arbitrary `cargo` commands using the managed installation.

* **Unit tests:** `./gradlew :flamegraph:cargoTest`
* **Lint:** `./gradlew :flamegraph:cargo --cmd "clippy"`
* **Format:** `./gradlew :flamegraph:cargo --cmd "fmt"`
* **Update crates:** `./gradlew :flamegraph:cargo --cmd "update"`

`Cargo.lock` is committed and the production build enforces it with `--locked`. After running
`update`, verify the changes and commit the updated `Cargo.lock` before building.

### Node & NPM

* **Type check:** `./gradlew :flamegraph:npx --cmd "tsc --noEmit"`
* **NPM commands:** `./gradlew :flamegraph:npm --cmd "install --save some-pkg"`
* **NPX commands:** `./gradlew :flamegraph:npx --cmd "vite"`

## Development

Development is facilitated with the Vite development server, which watches for changes to
TypeScript and Rust source files and performs hot-reloading on demand.

Start the dev server:

Run `/gradlew :flamegraph:serve`

This serves a live development version of the app at http://localhost:5173/

### Running tests

**Rust:**
```
./gradlew :flamegraph:cargoTest
```

**TypeScript (vitest, in `src/test/ts`):**
```
./gradlew :flamegraph:tsTest
```

Both test suites also run automatically as part of `./gradlew :flamegraph:check`.

**TypeScript (type check):**
```
./gradlew :flamegraph:npx --cmd "tsc --noEmit"
```
