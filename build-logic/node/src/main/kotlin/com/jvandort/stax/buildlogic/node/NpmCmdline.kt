package com.jvandort.stax.buildlogic.node

import com.github.gradle.node.npm.task.NpmTask
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.options.Option

/**
 * Runs an npm command given on the command line, such as --cmd="run lint".
 */
abstract class NpmCmdline : NpmTask() {

    @get:Input
    @get:Option(option = "cmd", description = "The npm command to run")
    abstract val cmd: Property<String>

    init {
        npmCommand.set(cmd.map { listOf(it.split("\\s+".toRegex()).first()) })
        args.set(cmd.map { it.split("\\s+".toRegex()).drop(1) })
    }

}
