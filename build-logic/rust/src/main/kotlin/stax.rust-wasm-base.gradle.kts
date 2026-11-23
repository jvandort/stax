import com.jvandort.stax.buildlogic.BuildRustWasm
import com.jvandort.stax.buildlogic.RustExtension
import com.jvandort.stax.buildlogic.UnpackTool
import com.jvandort.stax.buildlogic.WasmPackExtension
import com.jvandort.stax.buildlogic.common.Download
import com.jvandort.stax.buildlogic.common.Platform
import com.jvandort.stax.buildlogic.common.Platform.Architecture
import com.jvandort.stax.buildlogic.common.Platform.OperatingSystem

plugins {
    id("stax.rust-base")
}

// wasm-pack, downloaded into the build directory, which every BuildRustWasm task runs, and the
// WebAssembly target installed with Rust. Its wasm-opt is downloaded here too: wasm-pack would
// download it itself, but trusts only the CAs compiled into it, not the system's, so it can't
// through a TLS-intercepting proxy.

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

val unpackWasmPack = tasks.register<UnpackTool>("unpackWasmPack") {
    archive.set(downloadWasmPack.flatMap { it.destination })
    outputDir.set(layout.buildDirectory.dir("wasm-pack"))
    binFile = outputDir.map { it.file("wasm-pack") }
}

val downloadBinaryen = tasks.register<Download>("downloadBinaryen") {
    val platform = when (Platform.current) {
        Platform(OperatingSystem.MAC, Architecture.AARCH64) -> "arm64-macos"
        Platform(OperatingSystem.MAC, Architecture.X86_64) -> "x86_64-macos"
        Platform(OperatingSystem.LINUX, Architecture.AARCH64) -> "aarch64-linux"
        Platform(OperatingSystem.LINUX, Architecture.X86_64) -> "x86_64-linux"
        else -> error("Unsupported platform: ${Platform.current}")
    }
    url.set(wasmPack.binaryenVersion.map { "https://github.com/WebAssembly/binaryen/releases/download/$it/binaryen-$it-$platform.tar.gz" })
    destination.set(layout.buildDirectory.file("rust-downloads/binaryen.tar.gz"))
}

// The whole release, as wasm-opt on macOS loads lib/libbinaryen.dylib.
val unpackBinaryen = tasks.register<UnpackTool>("unpackBinaryen") {
    archive.set(downloadBinaryen.flatMap { it.destination })
    outputDir.set(layout.buildDirectory.dir("binaryen"))
    binFile = outputDir.map { it.file("bin/wasm-opt") }
}

tasks.withType<BuildRustWasm>().configureEach {
    wasmPackBin.set(unpackWasmPack.flatMap { it.binFile })
    wasmOptBin.set(unpackBinaryen.flatMap { it.binFile })
}
