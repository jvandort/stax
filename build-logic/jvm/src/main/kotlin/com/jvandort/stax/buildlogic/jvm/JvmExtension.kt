package com.jvandort.stax.buildlogic.jvm

import org.gradle.api.provider.Property

abstract class JvmExtension {

    /**
     * The class whose main method runs the project as an application, if it is one. It becomes
     * the jar's Main-Class, and what RunApplication tasks run.
     */
    abstract val mainClass: Property<String>

    /**
     * The Java version the classes target (javac --release), whatever JDK runs the build.
     */
    abstract val targetJvmVersion: Property<Int>

}
