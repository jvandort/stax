import com.jvandort.stax.buildlogic.BuildCargo
import com.jvandort.stax.buildlogic.common.DirectorySystemProperty
import com.jvandort.stax.buildlogic.jvm.RunApplication

plugins {
    id("stax.jvm")
    id("stax.rust-base")
    id("stax.node-rust")
    id("stax.node.playwright")
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

dependencies {
    api("org.jspecify:jspecify:1.0.1")
}

// Recordings and stacks files that the Rust, Java and browser tests share.
val testData = layout.projectDirectory.dir("src/test-data")

tasks.test {
    jvmArgumentProviders.add(objects.newInstance<DirectorySystemProperty>().apply {
        name = "stax.test-data.path"
        dir = testData
    })
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

// The page the Playwright tests open, made by the generator from the jar's classes.
val generateTestPage = tasks.register<GenerateFlamegraph>("generateTestPage") {
    embedded.from(
        layout.projectDirectory.file("src/browserTest/data/small-stacks.txt"),
        testData.file("cpu-sample.jfr"),
    )
    outputFile = layout.buildDirectory.file("browser-test/flamegraph.html")
}

tasks.playwrightTest {
    val page = generateTestPage.flatMap { it.outputFile }
    inputs.file(page)
    environment.put("STAX_TEST_PAGE", page.map { it.asFile.absolutePath })
}
