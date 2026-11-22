package com.jvandort.stax.buildlogic

import org.gradle.api.DefaultTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.ListProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.OutputFile
import org.gradle.api.tasks.TaskAction
import org.gradle.process.ExecOperations
import java.io.ByteArrayOutputStream
import javax.inject.Inject

/**
 * Installs a Rust toolchain, with rustup, into cargoHome and rustupHome.
 */
abstract class InstallRust : DefaultTask() {

    @get:Input
    abstract val rustVersion: Property<String>

    @get:InputFile
    abstract val rustupInitFile: RegularFileProperty

    @get:OutputDirectory
    abstract val cargoHome: DirectoryProperty

    @get:OutputDirectory
    abstract val rustupHome: DirectoryProperty

    /**
     * Where to write the toolchain's host target, what it builds for by default, as rustc -vV
     * reports it.
     */
    @get:OutputFile
    abstract val hostTargetFile: RegularFileProperty

    @get:Inject
    abstract val execOperations: ExecOperations

    /**
     * The Rust targets to install, besides the machine's own. Tasks don't install targets
     * themselves, since the toolchain is this task's output.
     */
    @get:Input
    abstract val targets: ListProperty<String>

    /**
     * The components to install, besides those in rustup's minimal profile, such as clippy.
     */
    @get:Input
    abstract val components: ListProperty<String>

    @TaskAction
    fun install() {
        execOperations.exec {
            environment(mapOf(
                "RUSTUP_HOME" to rustupHome.get().asFile.absolutePath,
                "CARGO_HOME" to cargoHome.get().asFile.absolutePath,
            ))
            executable = rustupInitFile.get().asFile.absolutePath
            args(
                "-y",
                "--no-modify-path",
                "--profile", "minimal",
                "--default-toolchain", rustVersion.get(),
            )
            if (targets.get().isNotEmpty()) {
                args("--target", targets.get().joinToString(","))
            }
            if (components.get().isNotEmpty()) {
                args("--component", components.get().joinToString(","))
            }
        }
        val output = ByteArrayOutputStream()
        execOperations.exec {
            environment(mapOf(
                "RUSTUP_HOME" to rustupHome.get().asFile.absolutePath,
                "CARGO_HOME" to cargoHome.get().asFile.absolutePath,
            ))
            executable = cargoHome.get().asFile.resolve("bin/rustc").absolutePath
            args("-vV")
            standardOutput = output
        }
        val host = output.toString().lineSequence().first { it.startsWith("host: ") }.removePrefix("host: ")
        hostTargetFile.get().asFile.writeText(host)
    }

}
