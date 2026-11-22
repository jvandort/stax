import com.jvandort.stax.buildlogic.BuildCargo
import com.jvandort.stax.buildlogic.common.DirectorySystemProperty
import com.jvandort.stax.buildlogic.jvm.RunApplication
import java.security.MessageDigest

plugins {
    id("stax.jvm")
    id("stax.rust-base")
    id("stax.node-rust")
    id("stax.node.playwright")
    id("java-test-fixtures")
    id("maven-publish")
    id("signing")
}

group = "com.jvandort"
version = "1.0.0"
description = "Self-contained, interactive flamegraphs for async-profiler stacks and JFR recordings."

rust {
    version = "1.99.1"
}

val buildCli = tasks.register<BuildCargo>("buildCli") {
    description = "Builds the Stax CLI binary."
    packageName = "stax"
    profile = "cli"
}

tasks.register<Tar>("packageCli") {
    description = "Packages the CLI for a GitHub release, with its SHA-256."
    compression = Compression.GZIP
    from(layout.settingsDirectory.file("LICENSE"))
    from(buildCli.flatMap { it.binary }) {
        filePermissions {
            unix("rwxr-xr-x")
        }
    }
    val version = project.version
    archiveFileName = buildCli.flatMap { it.target }.map { "stax-$version-$it.tar.gz" }
    destinationDirectory = layout.buildDirectory.dir("cli-dist")

    val shaFile = archiveFile.map { it.asFile.resolveSibling("${it.asFile.name}.sha256") }
    outputs.file(shaFile)
    doLast {
        val archive = archiveFile.get().asFile
        val digest = MessageDigest.getInstance("SHA-256").digest(archive.readBytes())
        val hex = digest.joinToString("") { "%02x".format(it) }
        shaFile.get().writeText("$hex  ${archive.name}\n")
    }
}

wasmPack {
    version = "0.15.0"
}

node {
    version = "26.13.0"
}

java {
    withSourcesJar()
    withJavadocJar()
}

jvm {
    mainClass = "com.jvandort.stax.FlamegraphGenerator"
    targetJvmVersion = 17
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

publishing {
    publications {
        register<MavenPublication>("maven") {
            from(components["java"])
            // The test fixtures are published as stax-<version>-test-fixtures.jar, for Gradle
            // builds to use as testFixtures("com.jvandort:stax:<version>"). A POM can't express
            // that variant, so Maven builds don't see them. That's expected, not worth a warning.
            suppressPomMetadataWarningsFor("testFixturesApiElements")
            suppressPomMetadataWarningsFor("testFixturesRuntimeElements")
            pom {
                name = project.name
                description = project.description
                url = "https://github.com/jvandort/stax"
                licenses {
                    license {
                        name = "Apache License, Version 2.0"
                        url = "https://www.apache.org/licenses/LICENSE-2.0.txt"
                    }
                }
                developers {
                    developer {
                        name = "Justin Van Dort"
                    }
                }
                scm {
                    connection = "scm:git:https://github.com/jvandort/stax.git"
                    url = "https://github.com/jvandort/stax"
                }
            }
        }
    }
}

// Signs with an ASCII-armored secret key passed in as Gradle properties (for example as
// ORG_GRADLE_PROJECT_signingKey), so no keyring is needed. Without a key, publishing
// locally still works, unsigned. Central rejects an unsigned upload.
signing {
    val signingKey = providers.gradleProperty("signingKey")
    if (signingKey.isPresent) {
        useInMemoryPgpKeys(signingKey.get(), providers.gradleProperty("signingPassword").orNull)
        sign(publishing.publications["maven"])
    }
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
