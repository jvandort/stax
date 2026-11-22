package com.jvandort.stax.buildlogic

import org.gradle.api.DefaultTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.tasks.InputDirectory
import org.gradle.api.tasks.Internal
import org.gradle.process.ExecOperations
import org.gradle.process.ExecSpec
import java.io.File
import javax.inject.Inject

/**
 * A task that runs commands with the Rust toolchain in cargoHome and rustupHome, in workingDir,
 * with cargo building into buildDir.
 */
abstract class CargoExec : DefaultTask() {

    @get:InputDirectory
    abstract val cargoHome: DirectoryProperty

    @get:InputDirectory
    abstract val rustupHome: DirectoryProperty

    @get:Internal
    abstract val buildDir: DirectoryProperty

    @get:InputDirectory
    abstract val workingDir: DirectoryProperty

    @get:Inject
    abstract val execOperations: ExecOperations

    /**
     * The path of one of the toolchain's commands, such as cargo or rustup.
     */
    protected fun toolchainPath(name: String): String =
        cargoHome.get().asFile.resolve("bin/$name").absolutePath

    /**
     * Runs a command with the toolchain's environment, in workingDir.
     */
    protected fun exec(configure: ExecSpec.() -> Unit) {
        val cargoHome = cargoHome.get().asFile.absolutePath
        execOperations.exec {
            environment(mapOf(
                "RUSTUP_HOME" to rustupHome.get().asFile.absolutePath,
                "CARGO_HOME" to cargoHome,
                "PATH" to "$cargoHome/bin${File.pathSeparator}${System.getenv("PATH")}",
                "CARGO_TARGET_DIR" to buildDir.get().asFile.absolutePath,
            ))
            workingDir(this@CargoExec.workingDir.get().asFile)
            configure()
        }
    }

}
