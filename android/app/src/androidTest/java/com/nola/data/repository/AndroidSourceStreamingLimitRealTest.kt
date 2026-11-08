package com.nola.data.repository

import androidx.test.platform.app.InstrumentationRegistry
import com.nola.domain.model.CustomDataSourceConfig
import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import java.io.File
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidSourceStreamingLimitRealTest {
    @Test
    fun chunkedResponseStopsBeforeDownloadingTheWholePayload() = runBlocking {
        val server = ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))
        val sent = AtomicLong()
        val workerError = AtomicReference<Throwable?>()
        val totalBytes = 32L * 1024 * 1024
        val worker = thread(isDaemon = true, name = "bounded-http-server") {
            try {
                server.accept().use { socket ->
                    socket.soTimeout = 5_000
                    val input = socket.getInputStream().bufferedReader()
                    check(input.readLine().startsWith("GET /large "))
                    while (!input.readLine().isNullOrEmpty()) { }
                    val output = socket.getOutputStream()
                    output.write(("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n" +
                        "Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n").toByteArray())
                    val chunk = ByteArray(16 * 1024) { 120 }
                    while (sent.get() < totalBytes) {
                        output.write("4000\r\n".toByteArray())
                        output.write(chunk)
                        output.write("\r\n".toByteArray())
                        output.flush()
                        sent.addAndGet(chunk.size.toLong())
                        Thread.sleep(2)
                    }
                    output.write("0\r\n\r\n".toByteArray())
                    output.flush()
                }
            } catch (_: IOException) {
                // The client is expected to close the oversized response.
            } catch (error: Throwable) {
                workerError.set(error)
            }
        }
        val client = HttpClient(CIO)
        val runtime = Runtime.getRuntime()
        fun heapBytes() = runtime.totalMemory() - runtime.freeMemory()
        val baseline = heapBytes()
        val peak = AtomicLong(baseline)
        val sampling = AtomicBoolean(true)
        val sampler = thread(isDaemon = true, name = "source-heap-sampler") {
            while (sampling.get()) {
                peak.accumulateAndGet(heapBytes(), ::maxOf)
                Thread.sleep(10)
            }
        }
        val started = System.nanoTime()
        try {
            val result = runCatching {
                AndroidDataSourceReader(client).read(CustomDataSourceConfig(
                    id = "large", name = "Large stream",
                    location = "http://127.0.0.1:${server.localPort}/large"
                ))
            }
            assertTrue("Expected size rejection, got ${result.exceptionOrNull()}",
                result.exceptionOrNull()?.message?.contains("1 MB") == true)
            worker.join(8_000)
            assertFalse("Response cleanup must stop the server stream", worker.isAlive)
            assertTrue("Downloaded the entire oversized body", sent.get() < totalBytes)
            assertTrue("Server fixture failed: ${workerError.get()}", workerError.get() == null)
        } finally {
            sampling.set(false)
            sampler.join(1_000)
            client.close()
            server.close()
            worker.join(1_000)
            val metrics = org.json.JSONObject()
                .put("attempted_payload_bytes", totalBytes)
                .put("server_written_bytes", sent.get())
                .put("elapsed_ms", (System.nanoTime() - started) / 1_000_000)
                .put("java_heap_baseline_bytes", baseline)
                .put("java_heap_sampled_peak_bytes", peak.get())
                .put("java_heap_sampled_growth_bytes", peak.get() - baseline)
                .put("sample_interval_ms", 10)
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            File(context.filesDir, "source-stream-limit-metrics.json").writeText(metrics.toString(2))
        }
    }
}
