// SPDX-License-Identifier: Apache-2.0
package com.nola.data.repository

import java.net.InetAddress
import java.net.ServerSocket
import kotlin.concurrent.thread

internal class SourceFixtureServer(private val body: String) : AutoCloseable {
    private val server = ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))
    private val worker = thread(name = "nola-android-source-fixture", isDaemon = true) {
        while (!server.isClosed) {
            val socket = try { server.accept() } catch (closed: java.net.SocketException) {
                if (server.isClosed) return@thread else throw closed
            }
            socket.use { client ->
                val reader = client.getInputStream().bufferedReader()
                while (reader.readLine()?.isNotEmpty() == true) Unit
                val payload = body.toByteArray(Charsets.UTF_8)
                client.getOutputStream().apply {
                    write("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: ${payload.size}\r\nConnection: close\r\n\r\n".toByteArray())
                    write(payload)
                    flush()
                }
            }
        }
    }

    val url: String = "http://127.0.0.1:${server.localPort}/context.json"

    override fun close() {
        runCatching { server.close() }
        worker.join(2_000)
    }
}
