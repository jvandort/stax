plugins {
    id("stax.vite")
    id("stax.rust-wasm")
}

// A Vite project that uses a Rust crate built to WebAssembly. Vite and Vitest find it through
// the RUST_WASM_DIR environment variable, relative to the project's directory: the environment
// is an input, and an absolute path would differ from one checkout to another.

val wasm = tasks.buildRustWasm.flatMap { it.outputDir }

listOf(tasks.buildVite, tasks.serveVite, tasks.tsTest).forEach { task ->
    task.configure {
        val projectDir = layout.projectDirectory.asFile
        inputs.dir(wasm)
        environment.put("RUST_WASM_DIR", wasm.map { it.asFile.relativeTo(projectDir).path })
    }
}

// tsconfig.json can't read env vars, and editors read it too, so it names the
// `build/wasm` location directly. typecheck takes the output as an input only so it runs
// after buildRustWasm and again when the bindings change. The cost is that tsc run outside
// Gradle checks against whatever build/wasm holds, which may be stale.
tasks.typecheck {
    inputs.dir(wasm)
}
