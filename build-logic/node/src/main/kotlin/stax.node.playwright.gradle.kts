import com.jvandort.stax.buildlogic.node.InstallPlaywrightBrowsers
import com.jvandort.stax.buildlogic.node.PlaywrightTest

plugins {
    id("stax.node-base")
}

// Runs the project's Playwright tests in Chromium's headless shell, as part of check. The
// project's package.json has to depend on @playwright/test (the tasks fail otherwise), and the
// project gives the tests whatever else they need.

val installChromium = tasks.register<InstallPlaywrightBrowsers>("installChromium") {
    description = "Installs Chromium's headless shell with the project's Playwright."
    dependsOn(tasks.npmInstall)
    packageLock = layout.projectDirectory.file("package-lock.json")
    installArgs = listOf("--only-shell", "chromium")
    browsersDir = layout.buildDirectory.dir("playwright-browsers")
}

val playwrightTest = tasks.register<PlaywrightTest>("playwrightTest") {
    description = "Runs the Playwright tests."
    browsersDir = installChromium.flatMap { it.browsersDir }
    resultsDir = layout.buildDirectory.dir("playwright-results")
}

tasks.check {
    dependsOn(playwrightTest)
}
