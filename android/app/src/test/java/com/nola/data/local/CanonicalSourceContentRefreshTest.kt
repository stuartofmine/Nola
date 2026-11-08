package com.nola.data.local

import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceReadMethod
import com.nola.domain.model.DataSourceReadRecipe
import com.nola.domain.model.DataSourceValidationContract
import java.io.File
import java.io.IOException
import java.net.URI
import java.nio.file.Files
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** Fault-injection tests for the Host coordinator, not a substitute for JNI/HTTP tests. */
class CanonicalSourceContentRefreshTest {
    @get:Rule
    val temporary = TemporaryFolder()

    @Test
    fun recordsIntentBeforeReadingAndDoesNotChangeConfiguration() = runBlocking {
        val application = ApplicationDouble()
        val directory = temporary.newFolder("success")
        val result = coordinator(application, directory).execute(source()) {
            assertEquals(listOf("request_refresh"), application.commands)
            assertEquals("pending", application.status())
            source().copy(cachedContent = "new content")
        }

        assertEquals(listOf("request_refresh", "record_read"), application.commands)
        assertEquals("succeeded", result.readState.getValue("status").jsonPrimitive.content)
        assertEquals("new content", application.contentFile().readText())
        assertEquals(1, directory.listFiles()!!.size)
    }

    @Test
    fun cancellationSettlesItsTokenAndStillPropagatesCancellation() = runBlocking {
        val application = ApplicationDouble()
        val directory = temporary.newFolder("cancel")
        val entered = CompletableDeferred<Unit>()
        val job = async {
            coordinator(application, directory).execute(source()) {
                entered.complete(Unit)
                awaitCancellation()
            }
        }
        withTimeout(5_000) { entered.await() }
        job.cancel(CancellationException("user cancelled"))
        withTimeout(5_000) { job.join() }

        assertTrue(job.isCancelled)
        assertEquals("failed", application.status())
        assertEquals("host_read_cancelled", application.lastResult!!.getValue("code").jsonPrimitive.content)
        assertEquals(listOf("request_refresh", "get", "record_read"), application.commands)
        assertTrue(directory.listFiles()!!.isEmpty())
    }

    @Test
    fun failedCommitRemovesUncommittedFileAndSettlesPendingRead() = runBlocking {
        val application = ApplicationDouble(failBeforeCommit = true)
        val directory = temporary.newFolder("failed-commit")
        val result = coordinator(application, directory).execute(source()) {
            source().copy(cachedContent = "uncommitted")
        }

        assertEquals("failed", result.readState.getValue("status").jsonPrimitive.content)
        assertEquals("host_read_failed", application.lastResult!!.getValue("code").jsonPrimitive.content)
        assertTrue(directory.listFiles()!!.isEmpty())
        assertEquals(listOf("request_refresh", "record_read", "get", "record_read"), application.commands)
    }

    @Test
    fun lostCommitResponsePreservesCommittedFileWithoutRepeatingRead() = runBlocking {
        val application = ApplicationDouble(failAfterCommit = true)
        val directory = temporary.newFolder("lost-response")
        var reads = 0
        val result = coordinator(application, directory).execute(source()) {
            reads += 1
            source().copy(cachedContent = "committed")
        }

        assertEquals(1, reads)
        assertEquals("succeeded", result.readState.getValue("status").jsonPrimitive.content)
        assertEquals("committed", application.contentFile().readText())
        assertEquals(listOf("request_refresh", "record_read", "get"), application.commands)
    }

    @Test
    fun staleFailureCannotSettleAnotherRequestsToken() = runBlocking {
        val application = ApplicationDouble()
        val directory = temporary.newFolder("superseded")
        val result = coordinator(application, directory).execute(source()) {
            application.replacePendingToken("newer-read")
            throw IOException("old request failed")
        }

        assertEquals("pending", result.readState.getValue("status").jsonPrimitive.content)
        assertEquals("newer-read", result.readState.getValue("read_token").jsonPrimitive.content)
        assertEquals(listOf("request_refresh", "get"), application.commands)
        assertTrue(directory.listFiles()!!.isEmpty())
    }

    @Test
    fun schemaChangeFailsReadRatherThanSilentlyRevalidating() = runBlocking {
        val application = ApplicationDouble()
        val directory = temporary.newFolder("schema")
        val original = source()
        coordinator(application, directory).execute(original) {
            original.copy(
                cachedContent = "unexpected shape",
                validationContract = original.validationContract!!.copy(schemaFields = listOf("different"))
            )
        }

        assertEquals("failed", application.status())
        assertTrue(application.lastResult!!.getValue("diagnostic").jsonPrimitive.content.contains("schema changed"))
        assertFalse(application.commands.any { it.contains("validation") || it == "update" })
        assertTrue(directory.listFiles()!!.isEmpty())
    }

    @Test
    fun unavailableCacheDirectorySettlesReadWithoutLeavingPendingIntent() = runBlocking {
        val application = ApplicationDouble()
        val directory = temporary.newFile("not-a-directory")
        coordinator(application, directory).execute(source()) { source().copy(cachedContent = "new") }

        assertEquals("failed", application.status())
        assertEquals(listOf("request_refresh", "get", "record_read"), application.commands)
        assertTrue(Files.isRegularFile(directory.toPath()))
    }

    private fun coordinator(application: ApplicationDouble, directory: File) =
        CanonicalSourceContentRefresh(CanonicalDataSourceClient(application::execute), directory)

    private fun source() = CustomDataSourceConfig(
        id = "source",
        name = "Source",
        location = "https://example.invalid/data",
        canonicalRevision = 4,
        validationContract = DataSourceValidationContract(
            sourceId = "source",
            readerDescription = "HTTP",
            format = "text",
            schemaFields = listOf("value"),
            validatedAt = 1_000,
            observedAt = 2_000,
            recordCount = 1,
            readRecipe = DataSourceReadRecipe(
                method = DataSourceReadMethod.HttpGet,
                operationId = "android.http.get",
                configurationKeys = listOf("url"),
                outputSchemaJson = "{\"type\":\"string\"}"
            )
        )
    )

    /** Implements only the three commands this coordinator may issue. */
    private class ApplicationDouble(
        private var failBeforeCommit: Boolean = false,
        private var failAfterCommit: Boolean = false
    ) {
        val commands = mutableListOf<String>()
        var lastResult: JsonObject? = null
            private set
        private var readState = buildJsonObject { put("status", "never_read") }
        private val base = Json.parseToJsonElement(
            """{
                "id":"source","name":"Source","kind":"http",
                "configuration":{"connector_id":"android.http","location_kind":"url",
                    "location":"https://example.invalid/data","parameters":{},"secret_references":[]},
                "access_policy":"conversation_opt_in",
                "authorization":{"state":"not_required","capability":null,"observed_at_ms":1000},
                "freshness_policy":{"maximum_age_minutes":60,"allow_stale_fallback":false},
                "capabilities":["read","context"],"privacy":"local-only",
                "created_at_ms":1000,"updated_at_ms":1000,"revision":4,
                "last_successful_read_at_ms":null,"deleted_at_ms":null,
                "default_enabled":false,"lifecycle":"ready",
                "validation":{"status":"succeeded","validation_token":null,"contract":null,
                    "diagnostic":null,"attempted_at_ms":1000,"completed_at_ms":1000}
            }"""
        ).jsonObject

        fun execute(envelope: JsonElement): JsonElement {
            val command = envelope.jsonObject.getValue("command").jsonObject
            val type = command.getValue("type").jsonPrimitive.content
            commands += type
            when (type) {
                "request_refresh" -> replacePendingToken("read-token")
                "get" -> Unit
                "record_read" -> {
                    val result = command.getValue("result").jsonObject
                    val succeeded = result.getValue("status").jsonPrimitive.content == "succeeded"
                    if (succeeded && failBeforeCommit) {
                        failBeforeCommit = false
                        throw IOException("commit unavailable")
                    }
                    lastResult = result
                    readState = result
                    if (succeeded && failAfterCommit) {
                        failAfterCommit = false
                        throw IOException("response lost after commit")
                    }
                }
                else -> error("Unexpected coordinator command: $type")
            }
            return JsonObject(base + ("read_state" to readState))
        }

        fun replacePendingToken(token: String) {
            readState = buildJsonObject {
                put("status", "pending")
                put("read_token", token)
            }
        }

        fun status() = readState.getValue("status").jsonPrimitive.content

        fun contentFile(): File = File(URI(readState.getValue("content_reference").jsonObject
            .getValue("uri").jsonPrimitive.content))
    }
}
