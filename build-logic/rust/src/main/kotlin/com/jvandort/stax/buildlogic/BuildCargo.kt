package com.jvandort.stax.buildlogic

import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.OutputFile
import org.gradle.api.tasks.TaskAction
import org.gradle.api.tasks.UntrackedTask

/**
 * Builds a package's binary, with cargo build.
 */
@UntrackedTask(because = "Cargo decides whether the binary is up to date")
abstract class BuildCargo : CargoExec() {

    /**
     * The package to build.
     */
    @get:Input
    abstract val packageName: Property<String>

    /**
     * The name of the binary cargo builds, the package's name unless Cargo.toml says otherwise.
     */
    @get:Input
    abstract val binaryName: Property<String>

    /**
     * The cargo profile to build with, such as release.
     */
    @get:Input
    abstract val profile: Property<String>

    /**
     * The Rust target triple to build for.
     */
    @get:Input
    abstract val target: Property<String>

    /**
     * Where cargo puts the binary, which follows from the other properties, so it can't be set.
     */
    @get:OutputFile
    abstract val binary: RegularFileProperty

    init {
        binaryName.convention(packageName)
        // Cargo names a profile's directory after it, except the built-in dev profile's.
        val profileDir = profile.map { if (it == "dev") "debug" else it }
        val path = target.zip(profileDir) { target, profileDir -> "$target/$profileDir" }
            .zip(binaryName) { dir, name -> "$dir/$name" }
        binary.set(buildDir.zip(path) { dir, path -> dir.file(path) })
        // We must use a property since Gradle does not attach producer tasks to output providers.
        // Prevent mutation so this acts like a property.
        binary.disallowChanges()
    }

    @TaskAction
    fun build() {
        exec {
            executable = toolchainPath("cargo")
            args("build", "--locked", "--profile", profile.get(), "-p", packageName.get(), "--target", target.get())
        }
    }

}
