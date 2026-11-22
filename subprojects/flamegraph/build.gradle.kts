import com.jvandort.stax.buildlogic.BuildCargo
import com.jvandort.stax.buildlogic.jvm.RunApplication

plugins {
    id("stax.jvm")
    id("stax.rust-base")
    id("stax.node-rust")
    id("java-test-fixtures")
}

rust {
    version = "1.99.1"
}

val buildCli = tasks.register<BuildCargo>("buildCli") {
    description = "Builds the Stax CLI binary."
    packageName = "stax"
    profile = "cli"
}

wasmPack {
    version = "0.15.0"
}

node {
    version = "26.13.0"
}

jvm {
    mainClass = "com.jvandort.stax.FlamegraphGenerator"
}

repositories {
    mavenCentral()
}

// Embed the vite-built index.html into the jar as a classpath resource so that
// FlamegraphGenerator can load it via Class.getResourceAsStream().
tasks.named<ProcessResources>("processResources") {
    from(tasks.buildVite) {
        include("index.html")
        into("com/jvandort/stax")
    }
}

/**
 * Writes a flamegraph page with FlamegraphGenerator, embedding the given profile files.
 */
@DisableCachingByDefault(because = "Large output file size")
abstract class GenerateFlamegraph : RunApplication() {

    /**
     * The profile files to embed, collapsed -stacks.txt or .jfr recordings, in order.
     */
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.NAME_ONLY)
    abstract val embedded: ConfigurableFileCollection

    @get:OutputFile
    abstract val outputFile: RegularFileProperty

    @Option(option = "embed", description = "The profile files (collapsed -stacks.txt or .jfr recordings) to embed")
    fun embed(paths: String) {
        embedded.from(paths.split(" "))
    }

    override fun arguments(): List<String> =
        embedded.files.map { it.absolutePath } + outputFile.get().asFile.absolutePath

}

tasks.register<GenerateFlamegraph>("buildDemo") {
    outputFile = layout.buildDirectory.file("demo/index.html")
}
