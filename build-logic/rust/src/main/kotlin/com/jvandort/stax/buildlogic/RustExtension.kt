package com.jvandort.stax.buildlogic

import org.gradle.api.file.DirectoryProperty
import org.gradle.api.provider.ListProperty
import org.gradle.api.provider.Property

abstract class RustExtension {

    /**
     * The version of the Rust toolchain to install.
     */
    abstract val version: Property<String>

    /**
     * The Rust targets to install, besides the machine's own and -PrustTarget's.
     */
    abstract val targets: ListProperty<String>

    /**
     * The cargo workspace's directory, src/main/rust by default.
     */
    abstract val workspaceDir: DirectoryProperty

}
