pluginManagement {
    includeBuild("build-logic")
}

// Not "stax": that's a subproject's name, and same-named projects confuse dependency resolution
// (https://github.com/gradle/gradle/issues/847).
rootProject.name = "stax-root"

fun subproject(name: String, dir: String = name) {
    include(name)
    project(":$name").projectDir = file("subprojects/$dir")
}

// Named stax, after the product, though it lives in flamegraph.
subproject("stax", "flamegraph")
