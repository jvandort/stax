import com.jvandort.stax.buildlogic.jvm.JvmExtension
import com.jvandort.stax.buildlogic.jvm.RunApplication

plugins {
    id("java-library")
}

val jvm = extensions.create<JvmExtension>("jvm")

tasks.javadoc {
    // Do not generate javadoc for internal classes, as they are not part of the public API.
    exclude("**/internal/**")
}

tasks.jar {
    manifest {
        attributes(mapOf("Main-Class" to jvm.mainClass))
    }
}

tasks.withType<JavaCompile>().configureEach {
    options.release.convention(jvm.targetJvmVersion)
}

tasks.withType<RunApplication>().configureEach {
    classpath.from(sourceSets.main.map { it.runtimeClasspath })
    mainClass.convention(jvm.mainClass)
}

testing.suites.named<JvmTestSuite>("test") {
    useJUnitJupiter()
}
