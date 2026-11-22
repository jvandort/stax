package com.jvandort.stax.buildlogic.common

import org.gradle.api.file.DirectoryProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.process.CommandLineArgumentProvider

/**
 * Wires a provider-defined directory to a JVM as a system property.
 */
abstract class DirectorySystemProperty : CommandLineArgumentProvider {

    @get:Input
    abstract val name: Property<String>

    @get:InputDirectory
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val dir: DirectoryProperty

    override fun asArguments(): List<String> = listOf("-D${name.get()}=${dir.get().asFile.absolutePath}")

}
