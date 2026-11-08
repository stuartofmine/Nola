package com.nola.data.remote.sse

import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import java.io.BufferedReader
import java.io.BufferedWriter
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SseClientEndToEndTest {
    @Test
    fun currentCorePublicationsDoNotAbortTheJournalStreamOrBecomeGrants() {
        val publications = listOf(
            "core_task_updated" to "\"task\":{}",
            "core_skill_runtime_updated" to "\"update\":{}",
            "core_capabilities_activated" to "\"capability_ids\":[\"code_mode\"]",
            "core_permission_projection_updated" to "\"permission_projection\":{\"revision\":0,\"pending\":[]}",
            "core_memory_runtime_projection_updated" to "\"memory_runtime_projection\":{}",
        )
        for ((type, payload) in publications) {
            // These Host publications have no trace; a UInt64 cursor is not
            // narrowed to signed Long just to consume a control hint.
            val frame = """{"type":"$type","protocol_version":1,"session_id":"conversation","cursor":18446744073709551615,$payload}"""
            val hint = decodeSsePayload(frame, null) as SseEvent.CoreProjectionHint
            assertEquals("conversation", hint.conversationId)
            assertEquals(type, hint.type)
            val encrypted = com.nola.data.remote.mobile_crypto.MobileCrypto.encryptJsonObject(
                Json.parseToJsonElement(frame) as kotlinx.serialization.json.JsonObject, "synthetic-test-token"
            ).copy(type = "encrypted")
            assertEquals(hint, decodeSsePayload(Json.encodeToString(com.nola.data.remote.mobile_crypto.EncryptedEnvelope.serializer(), encrypted), "synthetic-test-token"))
            assertTrue(decodeSsePayload("""{"type":"assistant_journal_committed_v1","session_id":"conversation","through":{}}""", null) is SseEvent.AssistantJournal)
            kotlin.test.assertFails { decodeSsePayload(frame.replace("\"session_id\":\"conversation\",", ""), null) }
            kotlin.test.assertFails { decodeSsePayload(frame.replace("\"protocol_version\":1", "\"protocol_version\":2"), null) }
        }
        kotlin.test.assertFails { decodeSsePayload("""{"type":"core_permission_projection_updated","protocol_version":1,"session_id":"conversation"}""", null) }
    }

    @org.junit.Test
    fun retiredBodyTraceAndDeltaPayloadCannotEnterControlStream() {
        for ((kind, payload) in listOf(
            "assistant.delta" to "{}", "assistant.reset" to "{}", "thinking.delta" to "{}",
            "tool.progress" to """{"delta":"not authority"}"""
        )) {
            val frame = """{"trace":{"schema_version":1,"trace_id":"trace","run_id":"run",
                "conversation_id":"conversation","turn_id":"turn","event_id":"event","sequence":1,
                "emitted_at":"2026-10-08T00:00:00Z","kind":"$kind","payload":$payload}}"""
            org.junit.Assert.assertThrows(IllegalStateException::class.java) { decodeSsePayload(frame, null) }
        }
    }
    @Test
    fun journalAndTerminalHintsUseProductionDecoderWithoutLegacySnapshot() {
        val journal = """{"type":"assistant_journal_committed_v1","source_host_id":"host","session_id":"conversation","through":{"identity":{"source_store_epoch":"epoch","conversation_id":"conversation","run_id":"run","assistant_message_id":"assistant"},"sequence":"18446744073709551615","record_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"""
        val event = decodeSsePayload(journal, null) as SseEvent.AssistantJournal
        assertTrue(event.envelope.getValue("through").toString().contains("18446744073709551615"))
        val terminal = """{"type":"assistant_run_terminal_committed_v1","source_host_id":"host","session_id":"conversation","run_id":"run","seal_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}"""
        assertTrue(decodeSsePayload(terminal, null) is SseEvent.AssistantTerminal)
    }

    @Test
    fun obsoleteFlattenedPartMirrorIsRejectedInsteadOfBecomingContent() {
        val payload = """{"protocol_version":1,"type":"core_part_updated","session_id":"conversation-1","turn_id":"turn-1","cursor":1,"revision":1,"projection_revision":1,"message_id":"assistant-1","part_id":"part-1","update":{"type":"created","part_id":"part-1"},"part":{"kind":"text","id":"part-1","message_id":"assistant-1","turn_id":"turn-1","text":"canonical","is_complete":false}}"""
        kotlin.test.assertFails { decodeSsePayload(payload, null) }
    }

    @Test
    fun rejectsNestedCoreLifecycleOnHostTransport() {
        val payload = """{"protocol_version":1,"session_id":"conversation-1","cursor":{"sequence":1},"revision":1,"event":{"type":"part_updated","turn_id":"turn-1","projection_revision":1,"update":{"type":"created","part_id":"part-1"},"part":{"kind":"text","id":"part-1","message_id":"assistant-1","turn_id":"turn-1","text":"old wire"}}}"""
        var failure: Throwable? = null
        try {
            decodeSsePayload(payload, null)
        } catch (error: Throwable) {
            failure = error
        }

        assertTrue(failure is IllegalStateException)
    }

    @Test
    fun rejectsLegacyProductEvents() {
        var failure: Throwable? = null
        try {
            decodeSsePayload("""{"type":"done"}""", null)
        } catch (error: Throwable) {
            failure = error
        }

        assertTrue(failure is IllegalStateException)
    }

    @Test
    fun realHttpStreamPreservesEventsAndStopsAtDoneFrame() = runTest {
        val requestBody = AtomicReference("")
        val requestSession = AtomicReference("")
        val executor = Executors.newSingleThreadExecutor()
        val server = ServerSocket(
            0,
            50,
            InetAddress.getByName("127.0.0.1")
        )
        executor.submit {
            server.accept().use { socket ->
                val input = BufferedReader(InputStreamReader(socket.getInputStream(), Charsets.UTF_8))
                val requestLine = input.readLine()
                check(requestLine.startsWith("POST /stream "))
                val headers = mutableMapOf<String, String>()
                while (true) {
                    val line = input.readLine() ?: break
                    if (line.isEmpty()) break
                    val separator = line.indexOf(':')
                    if (separator > 0) {
                        headers[line.substring(0, separator).lowercase()] =
                            line.substring(separator + 1).trim()
                    }
                }
                val bodyLength = headers["content-length"]?.toIntOrNull() ?: 0
                val body = CharArray(bodyLength)
                var offset = 0
                while (offset < bodyLength) {
                    val count = input.read(body, offset, bodyLength - offset)
                    if (count < 0) break
                    offset += count
                }
                requestBody.set(String(body, 0, offset))
                requestSession.set(headers["x-test-session"].orEmpty())

                val frames = listOf(
                    "data: {\"trace\":{\"schema_version\":1,\"trace_id\":\"trace-1\",\"run_id\":\"run-1\",\"conversation_id\":\"conversation-1\",\"message_id\":\"assistant-1\",\"turn_id\":\"turn-1\",\"event_id\":\"event-1\",\"sequence\":1,\"emitted_at\":\"2026-08-21T00:00:00Z\",\"kind\":\"phase.changed\",\"payload\":{\"phase\":\"Hello over HTTP\"}}}\n\n",
                    "data: [DONE]\n\n",
                    "data: {\"trace\":{\"schema_version\":1,\"trace_id\":\"trace-1\",\"run_id\":\"run-1\",\"conversation_id\":\"conversation-1\",\"message_id\":\"assistant-1\",\"turn_id\":\"turn-1\",\"event_id\":\"event-2\",\"sequence\":2,\"emitted_at\":\"2026-08-21T00:00:01Z\",\"kind\":\"phase.changed\",\"payload\":{\"phase\":\"must not be emitted\"}}}\n\n",
                )
                val contentLength = frames.sumOf { it.toByteArray(Charsets.UTF_8).size }
                val output = BufferedWriter(OutputStreamWriter(socket.getOutputStream(), Charsets.UTF_8))
                output.write(
                    "HTTP/1.1 200 OK\r\n" +
                        "Content-Type: text/event-stream\r\n" +
                        "Cache-Control: no-cache\r\n" +
                        "Connection: close\r\n" +
                        "Content-Length: $contentLength\r\n\r\n"
                )
                output.flush()
                frames.forEach { frame ->
                    runCatching {
                        output.write(frame)
                        output.flush()
                        Thread.sleep(8)
                    }
                }
            }
        }
        val client = HttpClient(CIO)

        try {
            val events = SseClient(client).stream(
                url = "http://127.0.0.1:${server.localPort}/stream",
                bodyJson = """{"message":"integration"}""",
                headers = mapOf("X-Test-Session" to "android-e2e")
            ).toList()

            assertEquals(1, events.size)
            assertEquals("Hello over HTTP", (events[0] as SseEvent.Trace).event.payload.phase)
            assertEquals("""{"message":"integration"}""", requestBody.get())
            assertEquals("android-e2e", requestSession.get())
        } finally {
            client.close()
            server.close()
            executor.shutdownNow()
        }
    }
}
