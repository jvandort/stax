rootProject.name = "stax"

fun subproject(name: String) {
    include(name)
    project(":$name").projectDir = file("subprojects/$name")
}

subproject("flamegraph")
