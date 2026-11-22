package com.jvandort.stax.buildlogic

import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.InputDirectory
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction

/**
 * Builds a crate to WebAssembly, with its JavaScript bindings, using wasm-pack.
 */
abstract class BuildRustWasm : CargoExec() {

    @get:InputFile
    abstract val wasmPackBin: RegularFileProperty

    /**
     * The directory of the crate to build.
     */
    @get:InputDirectory
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val crateDir: DirectoryProperty

    /**
     * The kind of JavaScript module to generate, as wasm-pack's --target, such as web.
     */
    @get:Input
    abstract val moduleTarget: Property<String>

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @TaskAction
    fun build() {
        exec {
            executable = wasmPackBin.get().asFile.absolutePath
            args(
                "build",
                "--target", moduleTarget.get(),
                "--out-dir", outputDir.get().asFile.absolutePath,
                crateDir.get().asFile.absolutePath,
                // wasm-pack passes what follows the crate's path on to cargo.
                "--locked",
            )
        }
    }

    companion object {

        /**
         * The Rust target wasm-pack builds for.
         */
        const val WASM_TARGET = "wasm32-unknown-unknown"

    }

}
