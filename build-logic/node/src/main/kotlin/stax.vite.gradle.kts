import com.github.gradle.node.npm.task.NpxTask

plugins {
    id("stax.node-base")
}

// A conventional Vite project: built with Vite into build/vite, and tested with Vitest and
// type-checked with tsc as part of check.

tasks.register<NpxTask>("buildVite") {
    description = "Builds the project with Vite into build/vite."
    dependsOn(tasks.npmInstall)
    val outputDir = layout.buildDirectory.dir("vite")
    command.set("vite")
    args.set(outputDir.map { listOf("build", "--outDir", it.asFile.absolutePath, "--emptyOutDir") })
    outputs.dir(outputDir)
    doNotTrackState("Its inputs aren't declared")
}

tasks.register<NpxTask>("serveVite") {
    description = "Runs Vite's development server."
    dependsOn(tasks.npmInstall)
    command.set("vite")
}

val tsTest = tasks.register<NpxTask>("tsTest") {
    description = "Runs the tests with Vitest."
    dependsOn(tasks.npmInstall)
    command.set("vitest")
    args.set(listOf("run"))
}

val typecheck = tasks.register<NpxTask>("typecheck") {
    description = "Type-checks everything tsconfig.json includes, with tsc."
    dependsOn(tasks.npmInstall)
    command.set("tsc")
    args.set(listOf("--noEmit"))
}

tasks.check {
    dependsOn(tsTest, typecheck)
}
