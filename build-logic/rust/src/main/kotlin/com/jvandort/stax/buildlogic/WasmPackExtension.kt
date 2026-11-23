package com.jvandort.stax.buildlogic

import org.gradle.api.provider.Property

abstract class WasmPackExtension {

    /**
     * The version of wasm-pack to download.
     */
    abstract val version: Property<String>

    /**
     * The version of binaryen, whose wasm-opt wasm-pack runs, as binaryen tags its releases, such as
     * version_117: the one this version of wasm-pack would download.
     */
    abstract val binaryenVersion: Property<String>

}
