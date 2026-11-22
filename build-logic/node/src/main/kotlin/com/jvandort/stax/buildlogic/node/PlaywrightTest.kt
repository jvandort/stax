package com.jvandort.stax.buildlogic.node

import com.github.gradle.node.npm.task.NpmTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.tasks.InputDirectory
import org.gradle.api.tasks.LocalState
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity

/**
 * Runs the project's Playwright tests, with playwright test, in the browsers in browsersDir. It
 * runs npm exec --no, so it fails, rather than download Playwright, if the project doesn't
 * depend on it.
 */
abstract class PlaywrightTest : NpmTask() {

    @get:InputDirectory
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val browsersDir: DirectoryProperty

    /**
     * Where Playwright writes what it keeps from failed tests, such as screenshots and traces.
     */
    @get:LocalState
    abstract val resultsDir: DirectoryProperty

    init {
        npmCommand.set(listOf("exec"))
        args.set(resultsDir.map { listOf("--no", "--", "playwright", "test", "--output", it.asFile.absolutePath) })
        environment.put("PLAYWRIGHT_BROWSERS_PATH", browsersDir.map { it.asFile.absolutePath })
    }

}
