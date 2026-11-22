import com.jvandort.stax.buildlogic.node.NpmCmdline
import com.jvandort.stax.buildlogic.node.NpxCmdline

plugins {
    id("base")
    id("com.github.node-gradle.node")
}

// Node.js, downloaded into the build directory, with npm and npx.

node {
    download = true
    npmInstallCommand = "ci"
    workDir = layout.buildDirectory.dir("node")
}

tasks.register<NpmCmdline>("npm") {
    description = "Runs an npm command given on the command line, such as --cmd=\"run lint\"."
}

tasks.register<NpxCmdline>("npx") {
    description = "Runs an npx command given on the command line, such as --cmd=\"tsc --version\"."
}

tasks.clean {
    delete(layout.projectDirectory.dir("node_modules"))
}
