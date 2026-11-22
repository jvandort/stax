package com.jvandort.stax.buildlogic.jvm

import org.gradle.api.DefaultTask
import org.gradle.api.file.ConfigurableFileCollection
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Classpath
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.TaskAction
import org.gradle.process.ExecOperations
import org.gradle.work.DisableCachingByDefault
import javax.inject.Inject

/**
 * Runs mainClass on classpath, with the arguments the subclass makes.
 */
@DisableCachingByDefault(because = "What the application does decides whether caching is worth it")
abstract class RunApplication : DefaultTask() {

    @get:Classpath
    abstract val classpath: ConfigurableFileCollection

    @get:Input
    abstract val mainClass: Property<String>

    @get:Inject
    abstract val execOperations: ExecOperations

    /**
     * The application's arguments, made from the subclass's inputs and outputs when the task runs.
     */
    protected abstract fun arguments(): List<String>

    @TaskAction
    fun run() {
        execOperations.javaexec {
            classpath(this@RunApplication.classpath)
            mainClass.set(this@RunApplication.mainClass)
            args(arguments())
        }
    }

}
