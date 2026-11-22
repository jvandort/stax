package com.jvandort.stax.buildlogic

import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.TaskAction
import java.io.ByteArrayOutputStream

/**
 * Checks that every package in the workspace has the expected version, as cargo tree lists them.
 */
abstract class CheckCargoVersion : CargoExec() {

    @get:Input
    abstract val expectedVersion: Property<String>

    @TaskAction
    fun check() {
        val output = ByteArrayOutputStream()
        exec {
            executable = toolchainPath("cargo")
            args("tree", "--locked", "--workspace", "--depth", "0", "--prefix", "none")
            standardOutput = output
        }
        // Such as stax v1.0.0 (/path/to/cli).
        val versions = Regex("""^(\S+) v(\S+) \(""", RegexOption.MULTILINE).findAll(output.toString())
            .associate { it.groupValues[1] to it.groupValues[2] }
        val wrong = versions.filterValues { it != expectedVersion.get() }
        check(wrong.isEmpty()) {
            "Expected every package in the workspace to be version ${expectedVersion.get()}, but " +
                wrong.entries.joinToString { (name, version) -> "$name is $version" }
        }
    }

}
