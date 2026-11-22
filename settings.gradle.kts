pluginManagement {
    includeBuild("build-logic")
}

plugins {
    id("com.gradleup.nmcp.settings").version("1.6.2")
}

// Not "stax": that's the published project's name, and same-named projects confuse dependency
// resolution (https://github.com/gradle/gradle/issues/847), which nmcp refuses to work with.
rootProject.name = "stax-root"

fun subproject(name: String, dir: String = name) {
    include(name)
    project(":$name").projectDir = file("subprojects/$dir")
}

// Named stax, for the artifact it publishes (com.jvandort:stax), though it lives in flamegraph.
subproject("stax", "flamegraph")

// Uploads the signed publications to the Central Portal (publishAggregationToCentralPortal).
// The credentials are a Central Portal user token, never the account password.
nmcpSettings {
    centralPortal {
        username = providers.gradleProperty("mavenCentralUsername")
        password = providers.gradleProperty("mavenCentralPassword")
        // Upload and validate only: the release is a button press on central.sonatype.com.
        publishingType = "USER_MANAGED"
    }
}
