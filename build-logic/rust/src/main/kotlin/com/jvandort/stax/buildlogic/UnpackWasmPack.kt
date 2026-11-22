package com.jvandort.stax.buildlogic

import org.gradle.api.DefaultTask
import org.gradle.api.file.ArchiveOperations
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.FileSystemOperations
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.file.RelativePath
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.Internal
import org.gradle.api.tasks.OutputFile
import org.gradle.api.tasks.TaskAction
import javax.inject.Inject

abstract class UnpackWasmPack : DefaultTask() {

    @get:InputFile
    abstract val archive: RegularFileProperty

    @get:Internal
    abstract val outputDir: DirectoryProperty

    @get:Inject
    abstract val archiveOperations: ArchiveOperations

    @get:Inject
    abstract val filesystemOperations: FileSystemOperations

    @get:OutputFile
    abstract val binFile: RegularFileProperty

    @TaskAction
    fun unpack() {
        filesystemOperations.copy {
            from(archive.map { archiveOperations.tarTree(it) }) {
                eachFile {
                    this.relativePath = RelativePath(true, *this.relativePath.segments.drop(1).toTypedArray<String>())
                    if (name == "wasm-pack") {
                        permissions {
                            user {
                                execute = true
                            }
                        }
                    }
                }
                this.includeEmptyDirs = false
            }
            into(outputDir)
        }
    }

}
