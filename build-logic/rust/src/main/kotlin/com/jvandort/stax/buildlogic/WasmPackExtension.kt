package com.jvandort.stax.buildlogic

import org.gradle.api.provider.Property

abstract class WasmPackExtension {

    /**
     * The version of wasm-pack to download.
     */
    abstract val version: Property<String>

}
