package com.jvandort.stax.buildlogic

import org.gradle.api.provider.ListProperty
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.TaskAction
import org.gradle.api.tasks.options.Option

/**
 * Runs a cargo command, such as test --workspace.
 */
abstract class Cargo : CargoExec() {

    @get:Input
    abstract val args: ListProperty<String>

    @Option(option = "cmd", description = "The cargo command to run, such as \"update -w\"")
    fun cmd(command: String) {
        args.set(command.split("\\s+".toRegex()))
    }

    @TaskAction
    fun run() {
        exec {
            executable = toolchainPath("cargo")
            args(this@Cargo.args.get())
        }
    }

}
