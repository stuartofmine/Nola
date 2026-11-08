package com.nola.data.local

import androidx.test.core.app.ApplicationProvider
import com.nola.core.AgentCoreNative
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import java.io.File

/** Real JNI calls; every canonical database lives under this test's unique cache directory. */
class AndroidSubmissionRecoveryNativeTest {
    @Test fun productionCancellationSubmissionPassesCurrentNativeSchema() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "cancel-contract-${System.nanoTime()}").also { it.mkdirs() }
        val database = File(root, "isolated.db")
        val handle = AgentCoreNative.nativeApplicationOpen(database.absolutePath.encodeToByteArray())
        assertTrue(handle != 0L)
        try {
            val request = buildJsonObject {
                put("protocol_version", AgentCoreNative.nativeApplicationProtocolVersion())
                put("client_id", "cancellation-contract")
                put("request_id", "cancel-missing-run")
                put("command", buildJsonObject {
                    put("type", "reduce_and_commit_active_submission")
                    put("arguments", buildJsonObject {
                        put("input", buildJsonObject {
                            put("conversation_id", "missing-conversation")
                            put("run_id", "missing-run")
                            put("transition_id", "cancel-transition")
                            put("invocation_config_digest", "0".repeat(64))
                            put("submission", com.nola.core.deviceAgentCancellationSubmission("user_requested"))
                            put("committed_at_ms", 1)
                        })
                    })
                })
            }
            val raw = requireNotNull(AgentCoreNative.nativeApplicationSubmit(handle, request.toString().encodeToByteArray()))
            val response = Json.parseToJsonElement(raw.decodeToString()).jsonObject
            // Missing business owner is a durable rejection. A malformed
            // cancellation instead fails before receipt admission (the bug).
            assertEquals(response.toString(), JsonPrimitive(true), response["ok"])
            val receipt = response.getValue("result").jsonObject
            assertEquals("rejected", receipt.getValue("status").jsonPrimitive.content)
            assertEquals("cancel-missing-run", receipt.getValue("request_id").jsonPrimitive.content)
        } finally {
            AgentCoreNative.nativeApplicationFree(handle)
            root.deleteRecursively()
        }
    }

    @Test fun pendingRecoveryDoesNotBlockCanonicalQueriesOrPermitNewWrites() {
        assertTrue("Real Rust JNI is required", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "pending-query-native-${System.nanoTime()}").also { it.mkdirs() }
        val database = File(root, "isolated.db")
        try {
            AndroidApplicationRuntime(context, database).use {
                it.putProductSetting("visible", "1", 1)
            }
            val decision = UnknownEffectRecoveryDecision("conversation", "run", "digest", "resolution",
                listOf(UnknownEffectRecoveryTarget("effect", "request", "attempt", "generation", "owner", "token", 1)),
                UnknownEffectRecoveryChoice(UnknownEffectRecoveryChoice.Kind.AuthorizeRiskRetry, "Explicit risk accepted"), 1)
            val journal = ApplicationSubmissionJournal(File(database.path + ".pending-commands.json"))
            val pending = journal.prepare(buildJsonObject {
                put("type", "resolve_unknown_effect_recovery"); put("input", Json.encodeToJsonElement(decision))
            }, AgentCoreNative.nativeApplicationProtocolVersion(), "fixed-recovery", null)
            AndroidApplicationRuntime(context, database).use { runtime ->
                assertEquals("1", runtime.executeQuery(buildJsonObject {
                    put("type", "get_product_setting"); put("key", "visible")
                }).jsonObject.getValue("value_json").jsonPrimitive.content)
                assertEquals(listOf(pending), journal.pending())
                assertEquals(decision, runtime.pendingUnknownEffectDecision("conversation")?.decision)
                try { runtime.putProductSetting("visible", "2", 2); fail("A write must not overtake pending recovery") }
                catch (_: AndroidApplicationRuntime.PendingMutation) {}
                assertEquals(listOf(pending), journal.pending())
            }
        } finally { root.deleteRecursively() }
    }

    @Test fun genericWriteAndLostReceiptRecoverThroughNativeColdReopen() {
        assertTrue("Real Rust JNI is required", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "submission-native-${System.nanoTime()}").also { it.mkdirs() }
        val database = File(root, "isolated.db")
        fun write(value: String, at: Long) = buildJsonObject {
            put("type", "put_product_setting")
            put("key", "native-recovery-test")
            put("value_json", value)
            put("updated_at_ms", at)
        }
        fun read() = buildJsonObject {
            put("type", "get_product_setting")
            put("key", "native-recovery-test")
        }
        try {
            AndroidApplicationRuntime(context, database).use {
                assertEquals("1", it.execute(write("1", 1)).jsonObject.getValue("value_json").jsonPrimitive.content)
            }
            AndroidApplicationRuntime(context, database).use {
                assertEquals("1", it.execute(read()).jsonObject.getValue("value_json").jsonPrimitive.content)
            }
            val journalFile = File(database.path + ".pending-commands.json")
            val journal = ApplicationSubmissionJournal(journalFile)
            val request = journal.prepare(write("2", 2), AgentCoreNative.nativeApplicationProtocolVersion(), "native-lost-receipt", null)
            val native = AgentCoreNative.nativeApplicationOpen(database.absolutePath.encodeToByteArray())
            assertTrue(native != 0L)
            val committed = try {
                val bytes = requireNotNull(AgentCoreNative.nativeApplicationSubmit(native, request.toString().encodeToByteArray()))
                val response = Json.parseToJsonElement(bytes.decodeToString()).jsonObject
                assertEquals("true", response.getValue("ok").jsonPrimitive.content)
                response.getValue("result").jsonObject.also {
                    assertEquals("committed", it.getValue("status").jsonPrimitive.content)
                }
                // Deliberately do not acknowledge the receipt in the journal: emulate loss after commit.
            } finally { AgentCoreNative.nativeApplicationFree(native) }
            assertEquals(listOf(request), ApplicationSubmissionJournal(journalFile).pending())
            AndroidApplicationRuntime(context, database).use { reopened ->
                assertEquals(listOf(committed), reopened.retryPendingCommands())
                assertEquals("2", reopened.execute(read()).jsonObject.getValue("value_json").jsonPrimitive.content)
                assertTrue(ApplicationSubmissionJournal(journalFile).pending().isEmpty())
                val receipt = reopened.execute(buildJsonObject {
                    put("type", "command_receipt")
                    put("request_id", "native-lost-receipt")
                })
                assertEquals(committed, receipt)
            }
            AndroidApplicationRuntime(context, database).use {
                assertTrue(it.retryPendingCommands().isEmpty())
                assertEquals("2", it.execute(read()).jsonObject.getValue("value_json").jsonPrimitive.content)
            }
        } finally { root.deleteRecursively() }
    }
}
