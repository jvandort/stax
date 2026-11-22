package com.jvandort.stax.buildlogic.node

import com.github.gradle.node.npm.task.NpxTask
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.options.Option

/**
 * Runs an npx command given on the command line, such as --cmd="tsc --noEmit".
 */
abstract class NpxCmdline : NpxTask() {

    @get:Input
    @get:Option(option = "cmd", description = "The npx command to run")
    abstract val cmd: Property<String>

    init {
        command.set(cmd.map { it.split("\\s+".toRegex()).first() })
        args.set(cmd.map { it.split("\\s+".toRegex()).drop(1) })
    }

}
