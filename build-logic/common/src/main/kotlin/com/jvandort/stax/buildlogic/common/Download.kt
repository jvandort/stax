package com.jvandort.stax.buildlogic.common

import org.gradle.api.DefaultTask
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.OutputFile
import org.gradle.api.tasks.TaskAction
import org.gradle.work.DisableCachingByDefault
import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse

@DisableCachingByDefault(because = "Not worth caching")
abstract class Download : DefaultTask() {

    @get:Input
    abstract val url: Property<String>

    @get:OutputFile
    abstract val destination: RegularFileProperty

    @TaskAction
    fun download() {
        val file = destination.get().asFile.also { it.parentFile.mkdirs() }
        val request = HttpRequest.newBuilder().uri(URI.create(url.get())).GET().build()
        val client = HttpClient.newBuilder().followRedirects(HttpClient.Redirect.NORMAL).build()
        client.send(request, HttpResponse.BodyHandlers.ofFile(file.toPath()))
    }

}
