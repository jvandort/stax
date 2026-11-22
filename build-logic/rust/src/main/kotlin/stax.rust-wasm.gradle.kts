import com.jvandort.stax.buildlogic.BuildRustWasm
import com.jvandort.stax.buildlogic.RustExtension

plugins {
    id("stax.rust-wasm-base")
}

// A crate in the workspace built to WebAssembly, the workspace's root package unless
// buildRustWasm's crateDir says otherwise. Its tests are the workspace's.

val rust = the<RustExtension>()

tasks.register<BuildRustWasm>("buildRustWasm") {
    description = "Builds the crate to WebAssembly, with its JavaScript bindings, into build/wasm."
    crateDir.convention(rust.workspaceDir)
    moduleTarget.convention("web")
    outputDir.convention(layout.buildDirectory.dir("wasm"))
}
