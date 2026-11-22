package com.jvandort.stax.buildlogic.common

/**
 * An operating system and CPU architecture, such as the one the build runs on.
 *
 * What a tool calls a platform is the tool's business, so mapping one to a download's name
 * belongs with that download.
 */
data class Platform(val os: OperatingSystem, val arch: Architecture) {

    enum class OperatingSystem {
        MAC, LINUX
    }

    enum class Architecture {
        AARCH64, X86_64
    }

    companion object {

        /**
         * The platform the build runs on.
         */
        val current: Platform = Platform(currentOs(), currentArch())

        private fun currentOs(): OperatingSystem {
            val name = System.getProperty("os.name").lowercase()
            return when {
                "mac" in name -> OperatingSystem.MAC
                "linux" in name -> OperatingSystem.LINUX
                else -> error("Unsupported operating system: $name. The build runs on macOS and Linux.")
            }
        }

        private fun currentArch(): Architecture {
            val name = System.getProperty("os.arch").lowercase()
            return when (name) {
                "aarch64", "arm64" -> Architecture.AARCH64
                "amd64", "x86_64" -> Architecture.X86_64
                else -> error("Unsupported architecture: $name")
            }
        }

    }

}
