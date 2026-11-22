package com.jvandort.stax.buildlogic.node

import com.github.gradle.node.npm.task.NpmTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.ListProperty
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity

/**
 * Installs browsers into browsersDir with the project's Playwright, which decides their versions.
 * It runs npm exec --no, so it fails, rather than download Playwright, if the project doesn't
 * depend on it.
 */
abstract class InstallPlaywrightBrowsers : NpmTask() {

    /**
     * The lock file, which pins the Playwright version.
     */
    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val packageLock: RegularFileProperty

    /**
     * What to install, as arguments to playwright install.
     */
    @get:Input
    abstract val installArgs: ListProperty<String>

    @get:OutputDirectory
    abstract val browsersDir: DirectoryProperty

    init {
        npmCommand.set(listOf("exec"))
        args.set(installArgs.map { listOf("--no", "--", "playwright", "install") + it })
        // locationOnly, since a value made from the task's own output would depend on the task.
        environment.put("PLAYWRIGHT_BROWSERS_PATH", browsersDir.locationOnly.map { it.asFile.absolutePath })
    }

}
