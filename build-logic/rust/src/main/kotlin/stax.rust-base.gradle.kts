import com.jvandort.stax.buildlogic.BuildCargo
import com.jvandort.stax.buildlogic.Cargo
import com.jvandort.stax.buildlogic.CargoExec
import com.jvandort.stax.buildlogic.InstallRust
import com.jvandort.stax.buildlogic.RustExtension
import com.jvandort.stax.buildlogic.CheckCargoVersion
import com.jvandort.stax.buildlogic.common.Download
import com.jvandort.stax.buildlogic.common.Platform
import com.jvandort.stax.buildlogic.common.Platform.Architecture
import com.jvandort.stax.buildlogic.common.Platform.OperatingSystem

plugins {
    id("base")
}

// A cargo workspace with a Rust toolchain installed into the build directory. Every cargo task
// runs in the workspace using the toolchain. The host target is used by default, or -PrustTarget,
// such as -PrustTarget=x86_64-unknown-linux-musl, overrides it.

val rust = extensions.create<RustExtension>("rust").apply {
    workspaceDir.convention(layout.projectDirectory.dir("src/main/rust"))
}

val rustTarget = providers.gradleProperty("rustTarget")

val downloadRustup = tasks.register<Download>("downloadRustup") {
    // rustup-init is published under the Rust target triple of the machine it runs on.
    val arch = when (Platform.current.arch) {
        Architecture.AARCH64 -> "aarch64"
        Architecture.X86_64 -> "x86_64"
    }
    val triple = when (Platform.current.os) {
        OperatingSystem.MAC -> "$arch-apple-darwin"
        OperatingSystem.LINUX -> "$arch-unknown-linux-gnu"
    }
    url.set("https://static.rust-lang.org/rustup/dist/$triple/rustup-init")
    destination.set(layout.buildDirectory.file("rust-downloads/rustup-init"))

    doLast {
        destination.get().asFile.setExecutable(true)
    }
}

val installRust = tasks.register<InstallRust>("installRust") {
    rustVersion.set(rust.version)
    rustupInitFile.set(downloadRustup.flatMap { it.destination })
    cargoHome.set(layout.buildDirectory.dir("cargo-home"))
    rustupHome.set(layout.buildDirectory.dir("rustup-home"))
    targets.set(rust.targets.zip(rustTarget.map { listOf(it) }.orElse(listOf())) { targets, target -> targets + target })
    hostTargetFile.set(layout.buildDirectory.file("rust-host-target.txt"))
    components.set(listOf("clippy", "rustfmt"))
}

tasks.withType<CargoExec>().configureEach {
    cargoHome.set(installRust.flatMap { it.cargoHome })
    rustupHome.set(installRust.flatMap { it.rustupHome })
    workingDir.set(rust.workspaceDir)
    buildDir.set(layout.buildDirectory.dir("cargo"))
}

tasks.withType<BuildCargo>().configureEach {
    target.convention(rustTarget.orElse(installRust.flatMap { it.hostTargetFile }.map { it.asFile.readText() }))
}

tasks.register<Cargo>("cargo") {
    description = "Runs a cargo command given on the command line, such as --cmd=\"update -w\"."
}

val cargoTest = tasks.register<Cargo>("cargoTest") {
    description = "Tests every package in the workspace."
    args = listOf("test", "--locked", "--workspace")
}

val checkCargoVersion = tasks.register<CheckCargoVersion>("checkCargoVersion") {
    description = "Checks that every package in the workspace has the project's version."
    expectedVersion.set(provider { project.version.toString() })
}

tasks.check {
    dependsOn(cargoTest, checkCargoVersion)
}
