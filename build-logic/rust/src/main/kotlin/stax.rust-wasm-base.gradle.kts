import com.jvandort.stax.buildlogic.BuildRustWasm
import com.jvandort.stax.buildlogic.RustExtension
import com.jvandort.stax.buildlogic.UnpackWasmPack
import com.jvandort.stax.buildlogic.WasmPackExtension
import com.jvandort.stax.buildlogic.common.Download
import com.jvandort.stax.buildlogic.common.Platform
import com.jvandort.stax.buildlogic.common.Platform.Architecture
import com.jvandort.stax.buildlogic.common.Platform.OperatingSystem

plugins {
    id("stax.rust-base")
}

// wasm-pack, downloaded into the build directory, which every BuildRustWasm task runs, and the
// WebAssembly target installed with Rust.

val wasmPack = extensions.create<WasmPackExtension>("wasmPack")

the<RustExtension>().targets.add(BuildRustWasm.WASM_TARGET)

val downloadWasmPack = tasks.register<Download>("downloadWasmPack") {
    // wasm-pack is published under Rust target triples. Its Linux builds are static (musl).
    val arch = when (Platform.current.arch) {
        Architecture.AARCH64 -> "aarch64"
        Architecture.X86_64 -> "x86_64"
    }
    val triple = when (Platform.current.os) {
        OperatingSystem.MAC -> "$arch-apple-darwin"
        OperatingSystem.LINUX -> "$arch-unknown-linux-musl"
    }
    url.set(wasmPack.version.map { "https://github.com/wasm-bindgen/wasm-pack/releases/download/v$it/wasm-pack-v$it-$triple.tar.gz" })
    destination.set(layout.buildDirectory.file("rust-downloads/wasm-pack.tar.gz"))
}

val unpackWasmPack = tasks.register<UnpackWasmPack>("unpackWasmPack") {
    archive.set(downloadWasmPack.flatMap { it.destination })
    outputDir.set(layout.buildDirectory.dir("wasm-pack"))
    binFile = outputDir.map { it.file("wasm-pack") }
}

tasks.withType<BuildRustWasm>().configureEach {
    wasmPackBin.set(unpackWasmPack.flatMap { it.binFile })
}
