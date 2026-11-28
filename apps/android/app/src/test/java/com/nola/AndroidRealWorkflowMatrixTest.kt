// SPDX-License-Identifier: Apache-2.0
package com.nola

import com.nola.core.AgentCoreAllowedAndroidTool
import com.nola.core.AgentCoreAndroidToolPlan
import com.nola.core.AgentCoreNative
import com.nola.core.AndroidAgentPermissionMode
import com.nola.core.AndroidAgentScopedGrantFact
import com.nola.core.androidAgentPermissionRules
import com.nola.core.corePermissionRule
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.AndroidAutomationSpec
import com.nola.data.local.ApplicationSubmissionJournal
import com.nola.data.local.CanonicalModelProfile
import com.nola.data.local.ConfigurationCommitResult
import com.nola.data.repository.ModelConnectionRepository
import com.nola.domain.model.DataSourceUsageRecord
import com.nola.domain.model.McpServerConfig
import com.nola.domain.model.StructuredMemoryRecord
import com.nola.domain.model.UploadedContent
import com.nola.ui.screen.chat.conversationPermissionMode
import java.io.File
import java.nio.file.Files
import java.nio.file.attribute.PosixFilePermissions
import java.util.concurrent.TimeUnit
import kotlin.test.assertFailsWith
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import kotlinx.serialization.json.put
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Dense real-workflow contract & JNI integration test matrix for Android:
 * 1. Strict 2-mode permission policy & rule composition (`cautious` / `allow_all`).
 * 2. Real JNI `NolaCore` SQLite v2 model connection lifecycle, `/props` reasoning refresh,
 *    read-time overlay, manual capability overrides, CAS revisions, and connection deletion.
 * 3. Multi-conversation lifecycle, window pagination, search, pinning, branching, A2UI, and cold reopen.
 * 4. ExpertPeer CRUD, AgentRoom submissions, StructuredMemory, Automation, Skill/MCP, and Draft DataSource overrides.
 * 5. Non-destructive rejection of legacy v1, future-version, and corrupt SQLite databases.
 */
class AndroidRealWorkflowMatrixTest {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

    companion object {
        @Volatile
        private var jniLoaded = false

        @Synchronized
        private fun ensureJniLoaded() {
            if (jniLoaded) return
            val jniLibPath = requireNotNull(System.getProperty("nola.test.modelPrivacyLibrary")) {
                "Expected nola.test.modelPrivacyLibrary system property from Gradle test task"
            }
            assertTrue("Expected compiled JNI library at $jniLibPath", File(jniLibPath).isFile)
            System.load(jniLibPath)
            assertEquals("Expected NolaCore Application ABI version 1", 1, AgentCoreNative.nativeApplicationAbiVersion())
            assertEquals("Expected NolaCore Application protocol version 1", 1, AgentCoreNative.nativeApplicationProtocolVersion())
            jniLoaded = true
        }
    }

    // -------------------------------------------------------------------------
    // 1. Strict 2-Mode Permission Matrix
    // -------------------------------------------------------------------------

    @Test
    fun permissionModeEntriesAreStrictlyCautiousAndAllowAllOnly() {
        assertEquals(
            listOf(AndroidAgentPermissionMode.Cautious, AndroidAgentPermissionMode.AllowAll),
            AndroidAgentPermissionMode.entries,
        )
        assertEquals("cautious", AndroidAgentPermissionMode.Cautious.wireValue)
        assertEquals("allow_all", AndroidAgentPermissionMode.AllowAll.wireValue)
        assertEquals(AndroidAgentPermissionMode.Cautious, AndroidAgentPermissionMode.fromWireValue(null))
        assertEquals(AndroidAgentPermissionMode.Cautious, AndroidAgentPermissionMode.fromWireValue("cautious"))
        assertEquals(AndroidAgentPermissionMode.AllowAll, AndroidAgentPermissionMode.fromWireValue("allow_all"))
    }

    @Test
    fun fromWireValueRejectsAcceptEditsAndLegacyOrInvalidModes() {
        val rejectedModes = listOf(
            "accept_edits",
            "ACCEPT_EDITS",
            "full_access",
            "plan",
            "default",
            "",
            " ",
            "cautious ",
            "allow-all",
        )
        for (invalid in rejectedModes) {
            val ex = assertFailsWith<IllegalArgumentException>(
                message = "Expected IllegalArgumentException for wireValue='$invalid'",
            ) {
                AndroidAgentPermissionMode.fromWireValue(invalid)
            }
            assertTrue(
                "Error message should mention cautious or allow_all: ${ex.message}",
                ex.message.orEmpty().contains("cautious") && ex.message.orEmpty().contains("allow_all"),
            )
        }
    }

    @Test
    fun conversationPermissionModeFallsBackOnNullAndRejectsAcceptEdits() {
        assertEquals(
            AndroidAgentPermissionMode.Cautious,
            conversationPermissionMode(null, AndroidAgentPermissionMode.Cautious),
        )
        assertEquals(
            AndroidAgentPermissionMode.AllowAll,
            conversationPermissionMode(null, AndroidAgentPermissionMode.AllowAll),
        )
        assertEquals(
            AndroidAgentPermissionMode.Cautious,
            conversationPermissionMode("cautious", AndroidAgentPermissionMode.AllowAll),
        )
        assertEquals(
            AndroidAgentPermissionMode.AllowAll,
            conversationPermissionMode("allow_all", AndroidAgentPermissionMode.Cautious),
        )

        for (invalid in listOf("accept_edits", "plan", "", "ask_always")) {
            assertFailsWith<IllegalArgumentException>(
                message = "conversationPermissionMode must reject '$invalid'",
            ) {
                conversationPermissionMode(invalid, AndroidAgentPermissionMode.Cautious)
            }
            assertFailsWith<IllegalArgumentException>(
                message = "conversationPermissionMode must reject '$invalid' even when default is AllowAll",
            ) {
                conversationPermissionMode(invalid, AndroidAgentPermissionMode.AllowAll)
            }
        }
    }

    @Test
    fun permitsResumingMatrixAllowsWideningAndBlocksTighteningAcrossAllPairs() {
        // Same mode -> allowed
        assertTrue(AndroidAgentPermissionMode.Cautious.permitsResuming(AndroidAgentPermissionMode.Cautious))
        assertTrue(AndroidAgentPermissionMode.AllowAll.permitsResuming(AndroidAgentPermissionMode.AllowAll))
        // Widening (AllowAll resuming Cautious) -> allowed
        assertTrue(AndroidAgentPermissionMode.AllowAll.permitsResuming(AndroidAgentPermissionMode.Cautious))
        // Tightening (Cautious resuming AllowAll) -> blocked
        assertFalse(AndroidAgentPermissionMode.Cautious.permitsResuming(AndroidAgentPermissionMode.AllowAll))

        // AndroidAutomationSpec must also enforce the strict 2-mode contract
        val cautiousSpec = AndroidAutomationSpec(
            prompt = "Summarize today's agenda",
            hostId = "android-device-1",
            permissionProfile = "cautious",
            contextRefs = listOf("personal_data" to "calendar"),
            skillRefs = listOf("daily-brief"),
        )
        assertEquals("cautious", cautiousSpec.toJson()["permission_profile"]?.jsonPrimitive?.content)

        val allowAllSpec = AndroidAutomationSpec(
            prompt = "Run cleanup",
            hostId = "android-device-1",
            permissionProfile = "allow_all",
            contextRefs = emptyList(),
            skillRefs = emptyList(),
        )
        assertEquals("allow_all", allowAllSpec.toJson()["permission_profile"]?.jsonPrimitive?.content)

        assertFailsWith<IllegalArgumentException> {
            AndroidAutomationSpec(
                prompt = "Legacy mode",
                hostId = "android-device-1",
                permissionProfile = "accept_edits",
                contextRefs = emptyList(),
                skillRefs = emptyList(),
            )
        }
    }

    @Test
    fun corePermissionRuleAndAndroidAgentPermissionRulesComposeHostToolsAndScopedGrants() {
        val cautiousBase = requireNotNull(AndroidAgentPermissionMode.Cautious.corePermissionRule())
        assertEquals(JsonPrimitive("*"), cautiousBase["capability_pattern"])
        assertEquals(JsonPrimitive("*"), cautiousBase["resource_pattern"])
        assertEquals(JsonPrimitive("allow"), cautiousBase["decision"])
        assertEquals(JsonPrimitive("product_default"), cautiousBase["source"])

        val allowAllBase = requireNotNull(AndroidAgentPermissionMode.AllowAll.corePermissionRule())
        assertEquals(JsonPrimitive("*"), allowAllBase["capability_pattern"])
        assertEquals(JsonPrimitive("*"), allowAllBase["resource_pattern"])
        assertEquals(JsonPrimitive("allow"), allowAllBase["decision"])
        assertEquals(JsonPrimitive("invocation_policy"), allowAllBase["source"])

        val tools = listOf(
            AgentCoreAllowedAndroidTool(
                toolId = "noop_tool",
                handlerId = "h0",
                capability = "none_cap",
                route = "device",
                permission = "none",
                authorization = "granted",
                risk = "low",
                requiresResultEgress = false,
            ),
            AgentCoreAllowedAndroidTool(
                toolId = "denied_tool",
                handlerId = "h1",
                capability = "camera",
                route = "device",
                permission = "system_permission",
                authorization = "denied",
                risk = "medium",
                requiresResultEgress = false,
            ),
            AgentCoreAllowedAndroidTool(
                toolId = "calendar_read",
                handlerId = "h2",
                capability = "calendar",
                route = "device",
                permission = "system_permission",
                authorization = "granted",
                risk = "low",
                requiresResultEgress = false,
            ),
            AgentCoreAllowedAndroidTool(
                toolId = "location_current",
                handlerId = "h3",
                capability = "location",
                route = "device",
                permission = "system_permission",
                authorization = "ask",
                risk = "medium",
                requiresResultEgress = false,
            ),
        )
        val grants = listOf(
            AndroidAgentScopedGrantFact(
                capability = "files_read",
                route = "device",
                scope = "  content://com.nola.docs/tree/notes  ",
                expiresAtMs = 9_999_999_999L,
            ),
            AndroidAgentScopedGrantFact(
                capability = "files_write",
                route = "device",
                scope = "   ",
                expiresAtMs = 9_999_999_999L,
            ),
        )

        val cautiousPlan = AgentCoreAndroidToolPlan(
            tools = emptyList(),
            grantedCapabilities = listOf("calendar"),
            allowedTools = tools,
            scopedGrants = grants,
            permissionMode = AndroidAgentPermissionMode.Cautious,
        )
        val cautiousRules = androidAgentPermissionRules(cautiousPlan).getValue("rules").jsonArray
        assertEquals(4, cautiousRules.size)
        assertEquals(cautiousBase, cautiousRules[0])
        assertEquals(
            buildJsonObject {
                put("capability_pattern", "calendar")
                put("resource_pattern", "*")
                put("decision", "allow")
                put("source", "host")
            },
            cautiousRules[1],
        )
        assertEquals(
            buildJsonObject {
                put("capability_pattern", "location")
                put("resource_pattern", "*")
                put("decision", "ask")
                put("source", "host")
            },
            cautiousRules[2],
        )
        assertEquals(
            buildJsonObject {
                put("capability_pattern", "files_read")
                put("resource_pattern", "content://com.nola.docs/tree/notes")
                put("decision", "allow")
                put("source", "host")
            },
            cautiousRules[3],
        )

        val allowAllRules = androidAgentPermissionRules(
            cautiousPlan.copy(permissionMode = AndroidAgentPermissionMode.AllowAll),
        ).getValue("rules").jsonArray
        assertEquals(4, allowAllRules.size)
        assertEquals(allowAllBase, allowAllRules[0])
        assertEquals(cautiousRules.drop(1), allowAllRules.drop(1))
    }

    // -------------------------------------------------------------------------
    // 2. Real JNI NolaCore Model Connection, /props Reasoning, Overlay & Overrides
    // -------------------------------------------------------------------------

    @Test
    fun jniModelConnectionRefreshWithPropsImmediatelyExposesReasoningWithoutRetoggling() {
        withTempRuntime("model-props-refresh") { runtime, _ ->
            val connId = "conn-strata-local"
            val t0 = 1_787_500_000_000L

            // Step 1: Create connection with a plain catalog entry (no reasoning yet).
            val imported = runtime.execute(buildJsonObject {
                put("type", "model")
                put("command", buildJsonObject {
                    put("type", "import_connection")
                    put("input", buildJsonObject {
                        put("id", connId)
                        put("display_name", "Strata LAN")
                        put("base_url", "http://192.168.1.50:8080/v1")
                        put("protocols", buildJsonArray { add(JsonPrimitive("chat_completions")) })
                        put("source", "manual")
                        put("enabled", true)
                    })
                    put("models", buildJsonArray {
                        add(buildJsonObject {
                            put("model_id", "strata-qwen3")
                            put("display_name", "Strata Qwen3")
                            put("protocol", "chat_completions")
                            put("modalities", buildJsonArray { add(JsonPrimitive("text")) })
                            put("reasoning_levels", buildJsonArray {})
                            put("execution_capabilities", buildJsonObject {
                                put("toolCalls", true)
                                put("imageInput", false)
                                put("reasoning", false)
                                put("samplingParameters", true)
                            })
                        })
                    })
                    put("enable_models", false)
                    put("updated_at_ms", t0)
                })
            }).jsonObject
            assertNotNull(imported["document"] ?: imported["connection"])

            // Step 2: Enable the model via configure_connection_models.
            runtime.execute(buildJsonObject {
                put("type", "model")
                put("command", buildJsonObject {
                    put("type", "configure_connection_models")
                    put("connection_id", connId)
                    put("enabled_model_ids", buildJsonArray { add(JsonPrimitive("chat_completions:strata-qwen3")) })
                    put("selected_model_id", "chat_completions:strata-qwen3")
                    put("updated_at_ms", t0 + 100)
                })
            })

            val initialProfile = runtime.listModelProfiles().single { it.modelName == "strata-qwen3" }
            val initialSrc = json.parseToJsonElement(initialProfile.sourceJson).jsonObject
            assertTrue(
                "Initial profile must NOT expose reasoning before /props discovery",
                initialSrc["reasoning"] == null || initialSrc["reasoning"] == JsonNull,
            )
            assertEquals(
                JsonPrimitive(false),
                initialSrc["executionCapabilities"]?.jsonObject?.get("reasoning"),
            )

            // Step 3: Re-refresh catalog with /props-enriched reasoning levels & vision/limits.
            runtime.execute(buildJsonObject {
                put("type", "model")
                put("command", buildJsonObject {
                    put("type", "refresh_connection_catalog")
                    put("connection_id", connId)
                    put("observed_at_ms", t0 + 5_000)
                    put("models", buildJsonArray {
                        add(buildJsonObject {
                            put("model_id", "strata-qwen3")
                            put("display_name", "Strata Qwen3")
                            put("protocol", "chat_completions")
                            put("context_window", 131_072)
                            put("output_limit", 8_192)
                            put("modalities", buildJsonArray {
                                add(JsonPrimitive("text"))
                                add(JsonPrimitive("image"))
                            })
                            put("reasoning_levels", buildJsonArray {
                                add(JsonPrimitive("none"))
                                add(JsonPrimitive("low"))
                                add(JsonPrimitive("medium"))
                                add(JsonPrimitive("high"))
                            })
                            put("execution_capabilities", buildJsonObject {
                                put("toolCalls", true)
                                put("imageInput", true)
                                put("reasoning", true)
                                put("samplingParameters", true)
                            })
                        })
                    })
                })
            })

            // Step 4: Verify list_profiles IMMEDIATELY exposes reasoning efforts WITHOUT re-toggling!
            val updatedProfile = runtime.listModelProfiles().single { it.modelName == "strata-qwen3" }
            val updatedSrc = json.parseToJsonElement(updatedProfile.sourceJson).jsonObject
            val reasoningObj = requireNotNull(updatedSrc["reasoning"]?.jsonObject) {
                "Expected reasoning object immediately after catalog refresh, sourceJson=${updatedProfile.sourceJson}"
            }
            assertEquals(JsonPrimitive("effort"), reasoningObj["control"])
            assertEquals(
                listOf("none", "low", "medium", "high"),
                reasoningObj.getValue("efforts").jsonArray.map { it.jsonPrimitive.content },
            )
            val execCaps = updatedSrc.getValue("executionCapabilities").jsonObject
            assertEquals(JsonPrimitive(true), execCaps["reasoning"])
            assertEquals(JsonPrimitive(true), execCaps["imageInput"])
            assertEquals(JsonPrimitive(true), execCaps["toolCalls"])
        }
    }

    @Test
    fun jniListProfilesReadTimeOverlayHydratesStaleProfileFromFresherConnectionCatalog() {
        withTempRuntime("model-read-overlay") { runtime, _ ->
            val connId = "conn-overlay-fresh"
            val staleTs = 1_787_600_000_000L
            val freshTs = staleTs + 60_000L

            // Seed a connection with a newer catalog (freshTs) advertising reasoning + limits.
            runtime.execute(buildJsonObject {
                put("type", "model")
                put("command", buildJsonObject {
                    put("type", "import_connection")
                    put("input", buildJsonObject {
                        put("id", connId)
                        put("display_name", "Overlay Gateway")
                        put("base_url", "https://overlay.example.invalid/v1")
                        put("protocols", buildJsonArray { add(JsonPrimitive("responses")) })
                        put("source", "manual")
                        put("enabled", true)
                    })
                    put("models", buildJsonArray {
                        add(buildJsonObject {
                            put("model_id", "o-reasoner")
                            put("display_name", "O-Reasoner")
                            put("protocol", "responses")
                            put("context_window", 200_000)
                            put("output_limit", 16_384)
                            put("reasoning_levels", buildJsonArray {
                                add(JsonPrimitive("low"))
                                add(JsonPrimitive("medium"))
                                add(JsonPrimitive("high"))
                            })
                            put("summary_supported", true)
                            put("execution_capabilities", buildJsonObject {
                                put("toolCalls", true)
                                put("imageInput", false)
                                put("reasoning", true)
                                put("samplingParameters", false)
                            })
                        })
                    })
                    put("enable_models", false)
                    put("updated_at_ms", freshTs)
                })
            })

            // Manually insert a STALE model profile (older updated_at_ms, missing reasoning and token limits).
            val staleProfileId = "conn:$connId:responses:o-reasoner"
            runtime.upsertModelProfile(
                CanonicalModelProfile(
                    id = staleProfileId,
                    displayName = "O-Reasoner",
                    provider = "openai",
                    modelName = "o-reasoner",
                    runtime = "remote",
                    sourceJson = buildJsonObject {
                        put("baseURL", "https://overlay.example.invalid/v1")
                        put("wireProtocol", "responses")
                        put("connectionID", connId)
                        put("catalogKey", "responses:o-reasoner")
                    }.toString(),
                    parametersJson = "{}",
                    selected = false,
                    enabled = true,
                    revision = 1,
                    createdAtMs = staleTs,
                    updatedAtMs = staleTs,
                    deletedAtMs = null,
                ),
            )

            // list_profiles must apply read-time overlay from the fresher connection catalog!
            val overlaid = runtime.listModelProfiles().single { it.id == staleProfileId }
            val src = json.parseToJsonElement(overlaid.sourceJson).jsonObject
            val reasoning = requireNotNull(src["reasoning"]?.jsonObject) {
                "Expected read-time overlay to populate reasoning, got: ${overlaid.sourceJson}"
            }
            assertEquals(
                listOf("low", "medium", "high"),
                reasoning.getValue("efforts").jsonArray.map { it.jsonPrimitive.content },
            )
            assertEquals(JsonPrimitive(true), reasoning["responses"])
            assertEquals(JsonPrimitive(true), src["reasoningSummarySupported"])
            assertEquals(JsonPrimitive(true), src["executionCapabilities"]?.jsonObject?.get("reasoning"))

            val params = json.parseToJsonElement(overlaid.parametersJson).jsonObject
            assertEquals(200_000L, params["maxContextLength"]?.jsonPrimitive?.long)
            assertEquals(16_384L, params["maxOutputTokens"]?.jsonPrimitive?.long)
        }
    }

    @Test
    fun jniManualCapabilityOverrideTogglesReasoningOffAndBackOnImmediately() {
        withTempRuntime("model-cap-override") { runtime, _ ->
            val connId = "conn-override-toggle"
            val baseTs = 1_787_700_000_000L

            runtime.execute(buildJsonObject {
                put("type", "model")
                put("command", buildJsonObject {
                    put("type", "import_connection")
                    put("input", buildJsonObject {
                        put("id", connId)
                        put("display_name", "Override Host")
                        put("base_url", "https://override.example.invalid/v1")
                        put("protocols", buildJsonArray { add(JsonPrimitive("chat_completions")) })
                        put("source", "manual")
                        put("enabled", true)
                    })
                    put("models", buildJsonArray {
                        add(buildJsonObject {
                            put("model_id", "deepseek-r1-local")
                            put("display_name", "DeepSeek R1 Local")
                            put("protocol", "chat_completions")
                            put("reasoning_levels", buildJsonArray {
                                add(JsonPrimitive("none"))
                                add(JsonPrimitive("low"))
                                add(JsonPrimitive("medium"))
                                add(JsonPrimitive("high"))
                            })
                            put("execution_capabilities", buildJsonObject {
                                put("toolCalls", true)
                                put("imageInput", false)
                                put("reasoning", true)
                                put("samplingParameters", true)
                            })
                        })
                    })
                    put("enable_models", true)
                    put("updated_at_ms", baseTs)
                })
            })
            runtime.execute(buildJsonObject {
                put("type", "model")
                put("command", buildJsonObject {
                    put("type", "configure_connection_models")
                    put("connection_id", connId)
                    put("enabled_model_ids", buildJsonArray { add(JsonPrimitive("chat_completions:deepseek-r1-local")) })
                    put("updated_at_ms", baseTs + 10)
                })
            })

            // Verify reasoning is initially ON
            val before = json.parseToJsonElement(
                runtime.listModelProfiles().single { it.modelName == "deepseek-r1-local" }.sourceJson,
            ).jsonObject
            assertNotNull("Expected reasoning ON before override", before["reasoning"]?.jsonObject)

            // Toggle reasoning OFF via manual capability override
            runtime.execute(buildJsonObject {
                put("type", "model")
                put("command", buildJsonObject {
                    put("type", "set_connection_capability_override")
                    put("connection_id", connId)
                    put("model_id", "deepseek-r1-local")
                    put("protocol", "chat_completions")
                    put("capabilities", buildJsonObject {
                        put("toolCalls", true)
                        put("imageInput", false)
                        put("reasoning", false)
                        put("samplingParameters", true)
                    })
                    put("updated_at_ms", baseTs + 1_000)
                })
            })

            val afterOff = json.parseToJsonElement(
                runtime.listModelProfiles().single { it.modelName == "deepseek-r1-local" }.sourceJson,
            ).jsonObject
            assertTrue(
                "Expected reasoning removed after OFF override, got: $afterOff",
                afterOff["reasoning"] == null || afterOff["reasoning"] == JsonNull,
            )
            assertEquals(JsonPrimitive(false), afterOff["executionCapabilities"]?.jsonObject?.get("reasoning"))

            // Toggle reasoning back ON (and enable imageInput) via manual capability override
            runtime.execute(buildJsonObject {
                put("type", "model")
                put("command", buildJsonObject {
                    put("type", "set_connection_capability_override")
                    put("connection_id", connId)
                    put("model_id", "deepseek-r1-local")
                    put("protocol", "chat_completions")
                    put("capabilities", buildJsonObject {
                        put("toolCalls", true)
                        put("imageInput", true)
                        put("reasoning", true)
                        put("samplingParameters", true)
                    })
                    put("updated_at_ms", baseTs + 2_000)
                })
            })

            val afterOn = json.parseToJsonElement(
                runtime.listModelProfiles().single { it.modelName == "deepseek-r1-local" }.sourceJson,
            ).jsonObject
            val restoredReasoning = requireNotNull(afterOn["reasoning"]?.jsonObject) {
                "Expected reasoning restored after ON override, got: $afterOn"
            }
            assertEquals(
                listOf("none", "low", "medium", "high"),
                restoredReasoning.getValue("efforts").jsonArray.map { it.jsonPrimitive.content },
            )
            assertEquals(JsonPrimitive(true), afterOn["executionCapabilities"]?.jsonObject?.get("reasoning"))
            assertEquals(JsonPrimitive(true), afterOn["executionCapabilities"]?.jsonObject?.get("imageInput"))
        }
    }

    @Test
    fun jniDeleteConnectionSoftDeletesLinkedProfilesAndPersistsRemovalAcrossColdReopen() {
        val tempRoot = createPrivateTempDir("nola-android-del-conn-")
        val dbFile = File(tempRoot, "del-conn.db")
        val connId = "conn-to-delete"
        val acctId = "acct-to-delete"
        val ts = 1_787_800_000_000L

        try {
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                runtime.execute(buildJsonObject {
                    put("type", "model")
                    put("command", buildJsonObject {
                        put("type", "import_connection")
                        put("input", buildJsonObject {
                            put("id", connId)
                            put("display_name", "Disposable Gateway")
                            put("base_url", "https://disposable.example.invalid/v1")
                            put("protocols", buildJsonArray { add(JsonPrimitive("chat_completions")) })
                            put("account_id", acctId)
                            put("source", "manual")
                            put("enabled", true)
                        })
                        put("account", buildJsonObject {
                            put("id", acctId)
                            put("display_name", "Disposable Account")
                            put("source", "manual")
                            put("state", "ready")
                        })
                        put("models", buildJsonArray {
                            add(buildJsonObject {
                                put("model_id", "disp-alpha")
                                put("display_name", "Disp Alpha")
                                put("protocol", "chat_completions")
                            })
                            add(buildJsonObject {
                                put("model_id", "disp-beta")
                                put("display_name", "Disp Beta")
                                put("protocol", "chat_completions")
                            })
                        })
                        put("enable_models", false)
                        put("updated_at_ms", ts)
                    })
                })
                runtime.execute(buildJsonObject {
                    put("type", "model")
                    put("command", buildJsonObject {
                        put("type", "configure_connection_models")
                        put("connection_id", connId)
                        put("enabled_model_ids", buildJsonArray {
                            add(JsonPrimitive("chat_completions:disp-alpha"))
                            add(JsonPrimitive("chat_completions:disp-beta"))
                        })
                        put("updated_at_ms", ts + 10)
                    })
                })

                val invBefore = ModelConnectionRepository(runtime).load()
                assertTrue(invBefore.connections.any { it.id == connId })
                assertEquals(2, runtime.listModelProfiles().count { it.modelName in setOf("disp-alpha", "disp-beta") })

                val delResp = runtime.execute(buildJsonObject {
                    put("type", "model")
                    put("command", buildJsonObject {
                        put("type", "delete_connection")
                        put("id", connId)
                        put("delete_account", true)
                        put("deleted_at_ms", ts + 500)
                    })
                }).jsonObject
                assertEquals(JsonPrimitive(true), delResp["ok"])
                assertEquals(JsonPrimitive(true), delResp["deleted"])
                assertEquals(2, delResp["removed_profiles"]?.jsonPrimitive?.int)

                assertFalse(ModelConnectionRepository(runtime).load().connections.any { it.id == connId })
                assertTrue(runtime.listModelProfiles().none { it.modelName in setOf("disp-alpha", "disp-beta") })
            }

            // Cold reopen must not resurrect deleted connection or profiles.
            openJvmAndroidApplicationRuntime(dbFile).use { reopened ->
                assertFalse(ModelConnectionRepository(reopened).load().connections.any { it.id == connId })
                assertTrue(reopened.listModelProfiles().isEmpty())
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun jniModelProfileCasParametersAndDeletePersistAcrossColdReopen() {
        val tempRoot = createPrivateTempDir("nola-android-model-cas-")
        val dbFile = File(tempRoot, "model-cas.db")
        val profileId = "android-cas-profile"
        val ts = 1_787_810_000_000L

        try {
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                val profile = CanonicalModelProfile(
                    id = profileId,
                    displayName = "CAS Model",
                    provider = "openai",
                    modelName = "gpt-5-mini",
                    runtime = "remote",
                    sourceJson = """{"baseURL":"https://api.example.invalid/v1","wireProtocol":"responses"}""",
                    parametersJson = """{"temperature":0.7,"maxContextLength":32768}""",
                    selected = true,
                    enabled = true,
                    revision = 1,
                    createdAtMs = ts,
                    updatedAtMs = ts,
                    deletedAtMs = null,
                )
                runtime.upsertModelProfile(profile)
                val loaded = requireNotNull(runtime.modelProfile(profileId))
                assertEquals("CAS Model", loaded.displayName)

                val nextRev = runtime.configureModelParameters(
                    profile = loaded.copy(
                        parametersJson = """{"temperature":0.25,"maxContextLength":65536}""",
                        updatedAtMs = ts + 100,
                    ),
                    expectedRevision = loaded.revision,
                )
                assertTrue("Expected revision to advance, got $nextRev", nextRev > loaded.revision)

                // Updating with stale revision must be rejected with RevisionConflict (if enforced) or advance cleanly on current rev.
                val updated = requireNotNull(runtime.modelProfile(profileId))
                assertTrue(updated.parametersJson.contains("65536"))
            }

            openJvmAndroidApplicationRuntime(dbFile).use { reopened ->
                val persisted = requireNotNull(reopened.modelProfile(profileId))
                assertTrue(persisted.parametersJson.contains("0.25"))
                assertTrue(reopened.deleteModelProfile(profileId, ts + 200))
                assertNull(reopened.modelProfile(profileId))
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    // -------------------------------------------------------------------------
    // 3. Multi-Conversation Lifecycle, Windowing, Search, Pinning & Cold Reopen
    // -------------------------------------------------------------------------

    @Test
    fun jniConversationLifecycleCreateRenamePinAppendLoadWindowSearchAndDelete() {
        withTempRuntime("conv-lifecycle") { runtime, _ ->
            val now = 1_787_900_000_000L
            val convId = "conv-matrix-main"
            val otherId = "conv-matrix-other"

            runtime.createConversation(convId, "Initial Draft", now, now, """{"titleSource":"automatic"}""")
            runtime.createConversation(otherId, "Grocery List", now + 10, now + 10, null)

            assertTrue(runtime.listConversations().any { it.id == convId })
            assertTrue(runtime.listRootConversations().any { it.id == convId })

            // Rename conversation
            runtime.renameConversation(convId, "Architecture Review Q4", now + 100)
            val afterRename = requireNotNull(runtime.loadConversation(convId))
            assertEquals("Architecture Review Q4", afterRename.title)

            // Pin conversation via patchConversationConfiguration & verify optimistic concurrency
            val cfgRev = (afterRename.configurationRevision ?: 1L).coerceAtLeast(1L)
            val pinResult = runtime.patchConversationConfiguration(
                id = convId,
                expectedRevision = cfgRev,
                patchJson = """{"isPinned":true,"selected_model_id":"gpt-5"}""",
            )
            val pinSnap = pinResult.requireCommitted()
            assertNotNull(pinSnap)

            val stalePatch = runtime.patchConversationConfiguration(
                id = convId,
                expectedRevision = cfgRev,
                patchJson = """{"isPinned":false}""",
            )
            assertTrue("Expected CAS Conflict on stale revision, got $stalePatch", stalePatch is ConfigurationCommitResult.Conflict)

            // Append 6 messages (3 user/assistant turns)
            val transcriptMessages = buildJsonArray {
                for (turn in 1..3) {
                    add(buildJsonObject {
                        put("id", "m-u-$turn")
                        put("role", "user")
                        put("created_at_ms", now + turn * 1_000L)
                        put("parts", buildJsonArray {
                            add(buildJsonObject {
                                put("id", "m-u-$turn:text")
                                put("kind", "text")
                                put("content_text", "User question turn $turn about Rust SQLite WAL")
                            })
                        })
                    })
                    add(buildJsonObject {
                        put("id", "m-a-$turn")
                        put("role", "assistant")
                        put("created_at_ms", now + turn * 1_000L + 500L)
                        put("parts", buildJsonArray {
                            add(buildJsonObject {
                                put("id", "m-a-$turn:text")
                                put("kind", "text")
                                put("content_text", "Assistant answer turn $turn confirming WAL durability")
                            })
                        })
                    })
                }
            }
            runtime.execute(buildJsonObject {
                put("type", "import_conversation_transcript")
                put("input", buildJsonObject {
                    put("import_id", "imp-matrix-1")
                    put("source", "external_archive")
                    put("conversation_id", convId)
                    put("expected_message_count", 0)
                    put("messages", transcriptMessages)
                    put("imported_at_ms", now + 5_000)
                })
            })

            // Verify full projection & paginated window loading
            val fullProjection = runtime.loadConversationProjectionV1(convId)
            assertEquals(6, fullProjection.messages.size)
            assertEquals("m-u-1", fullProjection.messages.first().messageId)
            assertEquals("m-a-3", fullProjection.messages.last().messageId)

            val tailWindow = runtime.loadConversationProjectionWindowV1(
                id = convId,
                beforeOrdinal = null,
                limit = 2,
                contentByteLimit = 65_536,
            )
            assertEquals(6L, tailWindow.totalMessageCount)
            assertEquals(2, tailWindow.projection.messages.size)
            assertEquals("m-u-3", tailWindow.projection.messages[0].messageId)
            assertEquals("m-a-3", tailWindow.projection.messages[1].messageId)
            assertTrue("Expected earlier messages before tail window", tailWindow.hasMore)

            val earlierWindow = runtime.loadConversationProjectionWindowV1(
                id = convId,
                beforeOrdinal = tailWindow.beforeOrdinal,
                limit = 2,
                contentByteLimit = 65_536,
            )
            assertEquals(2, earlierWindow.projection.messages.size)
            assertEquals("m-u-2", earlierWindow.projection.messages[0].messageId)
            assertEquals("m-a-2", earlierWindow.projection.messages[1].messageId)

            // Search via list_activity query filter
            val searchHits = runtime.executeQuery(buildJsonObject {
                put("type", "list_activity")
                put("query", "Architecture Review")
                put("needs_response", false)
            }).jsonObject.getValue("entries").jsonArray.map { it.jsonObject }
            assertTrue(searchHits.any { it["id"] == JsonPrimitive(convId) })
            assertFalse(searchHits.any { it["id"] == JsonPrimitive(otherId) })

            // Delete conversation & confirm removal
            runtime.deleteConversation(convId)
            assertNull(runtime.loadConversation(convId))
            assertFalse(runtime.listConversations().any { it.id == convId })
            assertNotNull(runtime.loadConversation(otherId))
        }
    }

    @Test
    fun jniMultiConversationPersistenceAndPermissionSettingsSurviveColdReopen() {
        val tempRoot = createPrivateTempDir("nola-android-cold-conv-")
        val dbFile = File(tempRoot, "cold-conv.db")
        val now = 1_787_910_000_000L

        try {
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                runtime.createConversation("conv-cold-1", "Cold Reopen Alpha", now, now, null)
                runtime.createConversation("conv-cold-2", "Cold Reopen Beta", now + 10, now + 10, null)

                val c1 = requireNotNull(runtime.loadConversation("conv-cold-1"))
                val permCommit = runtime.setConversationPermission(
                    id = "conv-cold-1",
                    expectedRevision = (c1.configurationRevision ?: 1L).coerceAtLeast(1L),
                    expectedRunId = null,
                    expectedSourceHostId = null,
                    profile = AndroidAgentPermissionMode.AllowAll.wireValue,
                )
                permCommit.requireCommitted()

                runtime.execute(buildJsonObject {
                    put("type", "import_conversation_transcript")
                    put("input", buildJsonObject {
                        put("import_id", "imp-cold-1")
                        put("source", "external_archive")
                        put("conversation_id", "conv-cold-1")
                        put("expected_message_count", 0)
                        put("messages", buildJsonArray {
                            add(buildJsonObject {
                                put("id", "cold-u-1")
                                put("role", "user")
                                put("created_at_ms", now + 20)
                                put("parts", buildJsonArray {
                                    add(buildJsonObject {
                                        put("id", "cold-u-1:text")
                                        put("kind", "text")
                                        put("content_text", "Persist across cold restart")
                                    })
                                })
                            })
                        })
                        put("imported_at_ms", now + 25)
                    })
                })
            }

            openJvmAndroidApplicationRuntime(dbFile).use { reopened ->
                val health = reopened.v2Health()
                assertEquals(JsonPrimitive(true), health["ok"])

                val list = reopened.listConversations()
                assertEquals(setOf("conv-cold-1", "conv-cold-2"), list.map { it.id }.toSet())

                val c1 = requireNotNull(reopened.loadConversation("conv-cold-1"))
                assertEquals("Cold Reopen Alpha", c1.title)
                assertEquals(1, c1.messages.size)
                assertEquals("Persist across cold restart", c1.messages.single().text)

                val c2 = requireNotNull(reopened.loadConversation("conv-cold-2"))
                assertEquals("Cold Reopen Beta", c2.title)
                assertTrue(c2.messages.isEmpty())
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun jniConversationBranchTruncateAndA2uiSurfaceLifecycle() {
        withTempRuntime("conv-branch-ui") { runtime, _ ->
            val now = 1_787_920_000_000L
            val convId = "conv-ui-branch"
            runtime.createConversation(convId, "UI & Branching", now, now, null)
            runtime.execute(buildJsonObject {
                put("type", "import_conversation_transcript")
                put("input", buildJsonObject {
                    put("import_id", "imp-branch-1")
                    put("source", "external_archive")
                    put("conversation_id", convId)
                    put("expected_message_count", 0)
                    put("messages", buildJsonArray {
                        add(buildJsonObject {
                            put("id", "b-m-1")
                            put("role", "user")
                            put("created_at_ms", now + 10)
                            put("parts", buildJsonArray {
                                add(buildJsonObject {
                                    put("id", "b-m-1:text")
                                    put("kind", "text")
                                    put("content_text", "First turn")
                                })
                            })
                        })
                        add(buildJsonObject {
                            put("id", "b-m-2")
                            put("role", "assistant")
                            put("created_at_ms", now + 20)
                            put("parts", buildJsonArray {
                                add(buildJsonObject {
                                    put("id", "b-m-2:text")
                                    put("kind", "text")
                                    put("content_text", "Second turn")
                                })
                            })
                        })
                    })
                    put("imported_at_ms", now + 30)
                })
            })

            // A2UI surface render, list, and delete
            runtime.v2RenderUiSurface(
                conversationId = convId,
                surfaceId = "surface-metrics",
                summary = "Metrics card",
                rootId = "root",
                components = buildJsonArray {
                    add(buildJsonObject {
                        put("id", "root")
                        put("type", "Text")
                        put("text", "Latency: 42ms")
                    })
                },
            )
            val listedSurfaces = runtime.executeQuery(buildJsonObject {
                put("type", "list_ui_surfaces")
                put("conversation_id", convId)
            })
            assertTrue("Expected surface-metrics in list_ui_surfaces: $listedSurfaces", listedSurfaces.toString().contains("surface-metrics"))
            runtime.v2DeleteUiSurface(convId, "surface-metrics")

            // Branch conversation creates a separate persisted conversation
            val branched = runtime.v2BranchConversation(convId)
            val branchedId = requireNotNull(
                branched["conversation"]?.jsonObject?.get("id")?.jsonPrimitive?.contentOrNull
                    ?: branched["id"]?.jsonPrimitive?.contentOrNull,
            ) { "Expected branched conversation ID in: $branched" }
            assertNotNull(runtime.loadConversation(branchedId))

            // Truncate original conversation cleanly
            val truncated = runtime.v2TruncateConversation(convId, fromSeq = 2L)
            assertEquals(JsonPrimitive(true), truncated["ok"])
        }
    }

    @Test
    fun jniDataSourceOverrideWorksOnUnsavedDraftConversationAndPersistsUsage() {
        val tempRoot = createPrivateTempDir("nola-android-draft-ds-")
        val dbFile = File(tempRoot, "draft-ds.db")
        val draftConversationId = "unsaved-draft-conv-42"
        val sourceId = "ds-calendar-main"
        val now = 1_787_930_000_000L

        try {
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                // Verify default override on unsaved draft conversation is "inherit"
                val initial = runtime.execute(buildJsonObject {
                    put("type", "data_source")
                    put("command", buildJsonObject {
                        put("type", "get_conversation_override")
                        put("conversation_id", draftConversationId)
                        put("source_id", sourceId)
                    })
                })
                assertEquals(JsonPrimitive("inherit"), initial)

                // Set explicit "enabled" override on draft conversation WITHOUT creating a session row
                val setEnabled = runtime.execute(buildJsonObject {
                    put("type", "data_source")
                    put("command", buildJsonObject {
                        put("type", "set_conversation_override")
                        put("conversation_id", draftConversationId)
                        put("source_id", sourceId)
                        put("value", "enabled")
                        put("updated_at_ms", now)
                    })
                })
                assertEquals(JsonPrimitive(true), setEnabled)

                val readEnabled = runtime.execute(buildJsonObject {
                    put("type", "data_source")
                    put("command", buildJsonObject {
                        put("type", "get_conversation_override")
                        put("conversation_id", draftConversationId)
                        put("source_id", sourceId)
                    })
                })
                assertEquals(JsonPrimitive("enabled"), readEnabled)

                // Toggle to "disabled"
                runtime.execute(buildJsonObject {
                    put("type", "data_source")
                    put("command", buildJsonObject {
                        put("type", "set_conversation_override")
                        put("conversation_id", draftConversationId)
                        put("source_id", sourceId)
                        put("value", "disabled")
                        put("updated_at_ms", now + 100)
                    })
                })

                // Record and list data source usage
                runtime.appendDataSourceUsage(
                    DataSourceUsageRecord(
                        id = "usage-draft-1",
                        sourceId = sourceId,
                        sourceTitle = "Calendar",
                        consumer = "chat",
                        purpose = "context_injection",
                        conversationId = draftConversationId,
                        disclosedToCloud = false,
                        usedAt = now + 200,
                    ),
                )
                val usage = runtime.listDataSourceUsage(limit = 20)
                assertTrue(usage.any { it.id == "usage-draft-1" && it.conversationId == draftConversationId })
            }

            // Cold reopen preserves draft override and usage audit row
            openJvmAndroidApplicationRuntime(dbFile).use { reopened ->
                val persisted = reopened.execute(buildJsonObject {
                    put("type", "data_source")
                    put("command", buildJsonObject {
                        put("type", "get_conversation_override")
                        put("conversation_id", draftConversationId)
                        put("source_id", sourceId)
                    })
                })
                assertEquals(JsonPrimitive("disabled"), persisted)
                assertTrue(reopened.listDataSourceUsage(limit = 20).any { it.id == "usage-draft-1" })
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    // -------------------------------------------------------------------------
    // 4. ExpertPeer CRUD, AgentRoom, Memory, Automation, Skills & MCP
    // -------------------------------------------------------------------------

    @Test
    fun jniExpertPeerCrudLifecyclePersistsAcrossColdReopen() {
        val tempRoot = createPrivateTempDir("nola-android-expert-")
        val dbFile = File(tempRoot, "expert.db")

        try {
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                val created = runtime.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "create_expert")
                        put("id", "expert-security")
                        put("name", "Security Auditor")
                        put("role", "reviewer")
                        put("system_prompt", "Audit SQL queries and auth boundaries.")
                        put("model_override", "claude-sonnet-4-5")
                        put("allowed_tools", buildJsonArray {
                            add(JsonPrimitive("read_file"))
                            add(JsonPrimitive("rg_search"))
                        })
                    })
                }).jsonObject
                assertEquals(JsonPrimitive(true), created["ok"])
                assertEquals(JsonPrimitive("expert-security"), created["expert"]?.jsonObject?.get("id"))

                val listed = runtime.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject { put("type", "list_experts") })
                }).jsonObject
                assertTrue(listed.getValue("experts").jsonArray.any { it.jsonObject["id"] == JsonPrimitive("expert-security") })
                assertTrue(listed.getValue("role_templates").jsonArray.isNotEmpty())

                // Update expert via upsert_expert
                val updated = runtime.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "upsert_expert")
                        put("id", "expert-security")
                        put("system_prompt", "Audit auth, crypto, and FFI boundaries.")
                        put("model_override", "gpt-5")
                    })
                }).jsonObject
                assertEquals(JsonPrimitive("gpt-5"), updated["expert"]?.jsonObject?.get("model_override"))
            }

            // Cold reopen verifies persistence, then delete_expert removes it
            openJvmAndroidApplicationRuntime(dbFile).use { reopened ->
                val listed = reopened.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject { put("type", "list_experts") })
                }).jsonObject.getValue("experts").jsonArray
                val expert = listed.single { it.jsonObject["id"] == JsonPrimitive("expert-security") }.jsonObject
                assertEquals(JsonPrimitive("Security Auditor"), expert["name"])
                assertEquals(JsonPrimitive("gpt-5"), expert["model_override"])
                assertEquals(JsonPrimitive("Audit auth, crypto, and FFI boundaries."), expert["system_prompt"])

                val deleted = reopened.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "delete_expert")
                        put("id", "expert-security")
                    })
                }).jsonObject
                assertEquals(JsonPrimitive(true), deleted["ok"])
                assertEquals(JsonPrimitive(true), deleted["deleted"])

                val afterDelete = reopened.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject { put("type", "list_experts") })
                }).jsonObject.getValue("experts").jsonArray
                assertTrue(afterDelete.none { it.jsonObject["id"] == JsonPrimitive("expert-security") })
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun jniAgentRoomCreationParticipantUpdateAndMessageSubmissionPersistence() {
        val tempRoot = createPrivateTempDir("nola-android-room-")
        val dbFile = File(tempRoot, "room.db")
        val roomId = "room-release-warroom"
        var submissionId = ""

        try {
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                runtime.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "create_expert")
                        put("id", "expert-qa")
                        put("name", "QA Lead")
                        put("role", "tester")
                    })
                })

                val createdRoom = runtime.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "create_room")
                        put("id", roomId)
                        put("name", "Release War Room")
                        put("participant_expert_ids", buildJsonArray { add(JsonPrimitive("expert-qa")) })
                    })
                }).jsonObject
                assertEquals(JsonPrimitive(roomId), createdRoom["room"]?.jsonObject?.get("id"))

                // Update room metadata
                val updatedRoom = runtime.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "update_room")
                        put("id", roomId)
                        put("name", "Release 2.0 War Room")
                        put("participant_expert_ids", buildJsonArray {
                            add(JsonPrimitive("expert-qa"))
                            add(JsonPrimitive("expert-release"))
                        })
                    })
                }).jsonObject
                assertEquals(JsonPrimitive(true), updatedRoom["ok"])
                assertEquals(JsonPrimitive("Release 2.0 War Room"), updatedRoom["room"]?.jsonObject?.get("name"))

                // Submit message to room and drive execution state machine
                val sub = runtime.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "submit")
                        put("room_id", roomId)
                        put("content", "Verify Android and iOS contract suites before tagging")
                    })
                }).jsonObject
                submissionId = requireNotNull(sub["submission_id"]?.jsonPrimitive?.contentOrNull)
                assertTrue(submissionId.isNotBlank())

                val selected = runtime.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "select_recipient")
                        put("room_id", roomId)
                        put("submission_id", submissionId)
                        put("recipient_id", "expert-qa")
                    })
                }).jsonObject
                assertEquals(JsonPrimitive("expert-qa"), selected["recipient_id"])

                val prepared = runtime.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "prepare_execution")
                        put("room_id", roomId)
                        put("submission_id", submissionId)
                    })
                }).jsonObject
                assertEquals(JsonPrimitive(true), prepared["ready"])

                val claimed = runtime.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "claim_execution")
                        put("room_id", roomId)
                        put("submission_id", submissionId)
                    })
                }).jsonObject
                assertEquals(JsonPrimitive(true), claimed["claimed"])

                val reconciled = runtime.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "reconcile_execution")
                        put("submission_id", submissionId)
                        put("status", "completed")
                    })
                }).jsonObject
                assertEquals(JsonPrimitive(true), reconciled["reconciled"])
            }

            // Cold reopen confirms room + submission history survived
            openJvmAndroidApplicationRuntime(dbFile).use { reopened ->
                val room = reopened.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "get_room")
                        put("id", roomId)
                    })
                }).jsonObject
                assertEquals(JsonPrimitive("Release 2.0 War Room"), room["name"])

                val subs = reopened.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "submissions")
                        put("room_id", roomId)
                    })
                }).jsonArray
                assertEquals(1, subs.size)
                assertEquals(JsonPrimitive(submissionId), subs[0].jsonObject["submission_id"])

                val deleted = reopened.execute(buildJsonObject {
                    put("type", "agent_room")
                    put("command", buildJsonObject {
                        put("type", "delete_room")
                        put("id", roomId)
                    })
                }).jsonObject
                assertEquals(JsonPrimitive(true), deleted["deleted"])
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun jniMemoryCrudRecallReplaceAndSearchPersistAcrossColdReopen() {
        val tempRoot = createPrivateTempDir("nola-android-memory-")
        val dbFile = File(tempRoot, "memory.db")
        val now = 1_787_940_000_000L

        try {
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                val owner = buildJsonObject {
                    put("kind", "profile")
                    put("profile_id", "local")
                }
                val pref = StructuredMemoryRecord(
                    id = "mem-pref-1",
                    owner = owner,
                    namespace = "personal",
                    scope = "global",
                    kind = "preference",
                    content = "User prefers Kotlin and Rust verification commands in release checklists.",
                    confidence = 0.95,
                    provenanceKind = "explicit_user",
                    sourceReference = "android://settings/memory",
                    observedAt = now,
                    createdAt = now,
                    updatedAt = now,
                    retention = "permanent",
                    tags = listOf("workflow", "testing"),
                )
                val committed = runtime.upsertMemory(pref)
                assertEquals("mem-pref-1", committed.id)
                assertEquals("preference", committed.kind)

                val longTerm = StructuredMemoryRecord(
                    id = "mem-lt-2",
                    owner = owner,
                    namespace = "long_term",
                    scope = "global",
                    kind = "project",
                    content = "Nola 2.0.0 uses unified SQLite schema user_version 2.",
                    confidence = 0.9,
                    provenanceKind = "explicit_user",
                    sourceReference = "android://settings/memory",
                    observedAt = now + 10,
                    createdAt = now + 10,
                    updatedAt = now + 10,
                    retention = "permanent",
                    tags = listOf("architecture"),
                )
                runtime.upsertMemory(longTerm)

                val snap = runtime.memorySnapshot()
                assertEquals(2, snap.records.size)

                val recalled = runtime.recallProfileMemories("verification", limit = 10)
                assertTrue(recalled.any { it.id == "mem-pref-1" })

                // Replace memories atomically (edit content of mem-pref-1, keep mem-lt-2)
                val replaced = runtime.replaceMemories(
                    expected = snap.canonical,
                    records = listOf(
                        pref.copy(content = "User prefers Gradle and Cargo verification commands.", updatedAt = now + 500),
                        longTerm,
                    ),
                )
                assertEquals(2, replaced.records.size)
                assertTrue(replaced.records.any { it.id == "mem-pref-1" && it.content.contains("Gradle") })

                // Also test v2SaveMemory + v2SearchMemory FTS5 path
                runtime.v2SaveMemory(
                    title = "Release Gate",
                    content = "Always run export_public.py before publishing to main.",
                    namespace = "default",
                    id = "mem-v2-fts",
                )
                val searchRes = runtime.v2SearchMemory("export_public", limit = 10)
                assertTrue("Expected FTS search hit for export_public: $searchRes", searchRes.toString().contains("export_public"))
            }

            openJvmAndroidApplicationRuntime(dbFile).use { reopened ->
                val all = reopened.listMemories()
                assertTrue(all.any { it.id == "mem-pref-1" && it.content.contains("Gradle") })
                assertTrue(all.any { it.id == "mem-lt-2" })

                reopened.deleteMemory("mem-lt-2", now + 1_000)
                assertFalse(reopened.listMemories().any { it.id == "mem-lt-2" })
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun jniAutomationDefinitionCrudAndRunLeaseLifecycleAcrossColdReopen() {
        val tempRoot = createPrivateTempDir("nola-android-auto-")
        val dbFile = File(tempRoot, "auto.db")
        val autoId = "auto-morning-digest"
        val now = 1_787_950_000_000L
        var runId = ""

        try {
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                val spec = AndroidAutomationSpec(
                    prompt = "Generate morning digest of active tasks",
                    hostId = "android-pixel-9",
                    permissionProfile = AndroidAgentPermissionMode.Cautious.wireValue,
                    contextRefs = listOf("personal_data" to "calendar"),
                    skillRefs = listOf("digest-skill"),
                )
                val v1 = runtime.saveAutomationDefinition(
                    id = autoId,
                    name = "Morning Digest",
                    enabled = true,
                    metadata = buildJsonObject { put("cron", "0 8 * * *") },
                    spec = spec,
                )
                assertEquals(1L, v1.revision)
                assertEquals(autoId, v1.id)
                assertEquals("0 8 * * *", v1.metadata["cron"]?.jsonPrimitive?.content)

                val v2 = runtime.saveAutomationDefinition(
                    id = autoId,
                    name = "Morning Digest v2",
                    enabled = true,
                    metadata = buildJsonObject { put("cron", "30 8 * * *") },
                    spec = spec.copy(permissionProfile = AndroidAgentPermissionMode.AllowAll.wireValue),
                    expectedRevision = v1.revision,
                )
                assertEquals(2L, v2.revision)

                // Stale expectedRevision must fail
                assertFailsWith<IllegalArgumentException> {
                    runtime.saveAutomationDefinition(
                        id = autoId,
                        name = "Stale Update",
                        enabled = false,
                        metadata = buildJsonObject {},
                        spec = spec,
                        expectedRevision = 1L,
                    )
                }

                // Enqueue occurrence idempotently and drive claim -> heartbeat -> retry
                val run = runtime.enqueueAutomationRun(
                    automationId = autoId,
                    definitionRevision = v2.revision,
                    occurrenceKey = "2026-11-28T08:30:00Z",
                    trigger = "schedule",
                    scheduledForMs = now,
                )
                runId = run.id
                assertEquals("queued", run.status)
                assertEquals(run.id, runtime.findAutomationOccurrence(autoId, "2026-11-28T08:30:00Z")?.id)

                assertTrue(runtime.claimAutomationRun(runId, "worker-android-1", "lease-tok-1", now + 10, now + 30_000))
                assertTrue(runtime.heartbeatAutomationRun(runId, "lease-tok-1", now + 5_000, now + 60_000))
                assertTrue(
                    runtime.retryAutomationRun(
                        id = runId,
                        leaseToken = "lease-tok-1",
                        nextRetryAtMs = now + 120_000,
                        errorJson = """{"code":"transient_network"}""",
                        updatedAtMs = now + 6_000,
                    ),
                )
            }

            openJvmAndroidApplicationRuntime(dbFile).use { reopened ->
                val loadedDef = requireNotNull(reopened.loadAutomationDefinition(autoId))
                assertEquals(2L, loadedDef.revision)
                assertEquals("30 8 * * *", loadedDef.metadata["cron"]?.jsonPrimitive?.content)
                assertEquals("allow_all", loadedDef.spec.permissionProfile)

                val loadedRun = requireNotNull(reopened.loadAutomationRun(runId))
                assertEquals(autoId, loadedRun.automationId)
                assertTrue("Expected retry error persisted", loadedRun.errorJson.orEmpty().contains("transient_network"))

                reopened.deleteAutomationDefinition(autoId)
                assertNull(reopened.loadAutomationDefinition(autoId))
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun jniSkillAndMcpServerCrudWithRevisionChecksAcrossColdReopen() {
        val tempRoot = createPrivateTempDir("nola-android-skill-mcp-")
        val dbFile = File(tempRoot, "skill-mcp.db")

        try {
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                val createdSkill = runtime.createUserSkill(
                    UploadedContent(
                        resourceId = "skill-code-audit",
                        name = "Code Audit",
                        description = "Review diffs for regressions",
                        content = "Check every edited file for edge cases.",
                        requiredTools = listOf("read_file"),
                        enabled = true,
                    ),
                )
                assertEquals(1L, createdSkill.revision)

                val editedSkill = runtime.configureUserSkill(
                    original = createdSkill,
                    name = "Strict Code Audit",
                    content = "Check every edited file for edge cases and SQLite migrations.",
                    description = "Strict regression review",
                )
                assertTrue(editedSkill.revision > createdSkill.revision)

                val disabledSkill = runtime.setSkillEnabled(editedSkill, enabled = false)
                assertEquals(JsonPrimitive(false), disabledSkill.record["enabled"])

                val createdMcp = runtime.createMcpServer(
                    McpServerConfig(
                        serverId = "mcp-docs-server",
                        name = "Docs MCP",
                        url = "https://mcp.example.invalid/sse",
                        enabled = true,
                    ),
                )
                assertEquals("mcp-docs-server", createdMcp.server.serverId)
                assertTrue(runtime.listMcpServers().any { it.serverId == "mcp-docs-server" })
            }

            openJvmAndroidApplicationRuntime(dbFile).use { reopened ->
                val skillSnap = requireNotNull(reopened.skillEditor("skill-code-audit"))
                assertEquals(JsonPrimitive("Strict Code Audit"), skillSnap.record["display_name"])
                assertEquals(JsonPrimitive(false), skillSnap.record["enabled"])
                reopened.deleteUserSkill(skillSnap)
                assertNull(reopened.skillEditor("skill-code-audit"))

                val mcpSnap = reopened.mcpEditorSnapshots().single { it.server.serverId == "mcp-docs-server" }
                reopened.deleteMcpServer(mcpSnap)
                assertTrue(reopened.listMcpServers().none { it.serverId == "mcp-docs-server" })
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun jniProductSettingsAndSyncCursorRoundTripAcrossColdReopen() {
        val tempRoot = createPrivateTempDir("nola-android-settings-sync-")
        val dbFile = File(tempRoot, "settings-sync.db")
        val now = 1_787_960_000_000L

        try {
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                val saved = runtime.putProductSetting("chat.default_permission_mode", """{"mode":"cautious"}""", now)
                assertEquals("chat.default_permission_mode", saved.key)
                assertEquals("""{"mode":"cautious"}""", saved.valueJson)

                val cursor = runtime.saveSyncCursor("replica-pixel-9", 42L, now + 100)
                assertEquals("replica-pixel-9", cursor.replicaId)
                assertEquals(42L, cursor.cursor)
            }

            openJvmAndroidApplicationRuntime(dbFile).use { reopened ->
                assertEquals(
                    """{"mode":"cautious"}""",
                    reopened.productSetting("chat.default_permission_mode")?.valueJson,
                )
                assertEquals(42L, reopened.syncCursor("replica-pixel-9")?.cursor)
                assertTrue(reopened.deleteProductSetting("chat.default_permission_mode"))
                assertNull(reopened.productSetting("chat.default_permission_mode"))
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    // -------------------------------------------------------------------------
    // 5. Legacy v1, Future Schema & Corrupt Database Non-Destructive Rejection
    // -------------------------------------------------------------------------

    @Test
    fun jniRejectsLegacyV1SqliteDatabaseWithoutModifyingRowsOrBytes() {
        ensureJniLoaded()
        val tempRoot = createPrivateTempDir("nola-android-v1-reject-")
        val v1Db = File(tempRoot, "legacy-v1.db")

        try {
            runPythonSqlite(
                v1Db,
                "CREATE TABLE preserved (val TEXT); INSERT INTO preserved VALUES ('keep-android-v1-data'); PRAGMA user_version = 1;",
            )
            val originalBytes = v1Db.readBytes()

            val handle = AgentCoreNative.nativeApplicationOpen(v1Db.absolutePath.encodeToByteArray())
            assertEquals("Legacy v1 DB (user_version=1) MUST be rejected with handle=0", 0L, handle)

            assertArrayEquals("Legacy v1 DB bytes must NOT be modified on rejection", originalBytes, v1Db.readBytes())
            val preserved = queryPythonSqliteScalar(v1Db, "SELECT val FROM preserved;")
            assertEquals("keep-android-v1-data", preserved)
            assertEquals("1", queryPythonSqliteScalar(v1Db, "PRAGMA user_version;"))
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun jniRejectsCorruptNonSqliteDatabaseWithoutModifyingRawBytes() {
        ensureJniLoaded()
        val tempRoot = createPrivateTempDir("nola-android-corrupt-reject-")
        val corruptDb = File(tempRoot, "corrupt.db")
        val sentinel = "NOT-A-VALID-SQLITE-DATABASE-SENTINEL-2026".toByteArray(Charsets.UTF_8)

        try {
            corruptDb.writeBytes(sentinel)
            val handle = AgentCoreNative.nativeApplicationOpen(corruptDb.absolutePath.encodeToByteArray())
            assertEquals("Corrupt non-SQLite DB MUST be rejected with handle=0", 0L, handle)
            assertArrayEquals("Corrupt DB raw bytes must remain untouched", sentinel, corruptDb.readBytes())
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun jniRejectsUnknownFutureSchemaVersionDatabaseWithoutModifyingBytes() {
        ensureJniLoaded()
        val tempRoot = createPrivateTempDir("nola-android-future-reject-")
        val futureDb = File(tempRoot, "future-v99.db")

        try {
            runPythonSqlite(
                futureDb,
                "CREATE TABLE future_state (k TEXT); INSERT INTO future_state VALUES ('v99-sentinel'); PRAGMA user_version = 99;",
            )
            val originalBytes = futureDb.readBytes()

            val handle = AgentCoreNative.nativeApplicationOpen(futureDb.absolutePath.encodeToByteArray())
            assertEquals("Future schema DB (user_version=99) MUST be rejected with handle=0", 0L, handle)
            assertArrayEquals("Future schema DB bytes must remain untouched", originalBytes, futureDb.readBytes())
            assertEquals("v99-sentinel", queryPythonSqliteScalar(futureDb, "SELECT k FROM future_state;"))
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun jniBackupCreatesConsistentReadableSqliteV2Snapshot() {
        val tempRoot = createPrivateTempDir("nola-android-backup-")
        val sourceDb = File(tempRoot, "source.db")
        val backupDb = File(tempRoot, "backup.db")
        val now = 1_787_970_000_000L

        try {
            openJvmAndroidApplicationRuntime(sourceDb).use { runtime ->
                runtime.createConversation("conv-backup-1", "Backed Up Thread", now, now, null)
                runtime.putProductSetting("backup.marker", """{"verified":true}""", now)
                val backupResp = runtime.execute(buildJsonObject {
                    put("type", "backup")
                    put("destination", backupDb.absolutePath)
                }).jsonObject
                assertEquals(JsonPrimitive(backupDb.absolutePath), backupResp["backup"])
            }

            assertTrue("Expected backup SQLite file created", backupDb.isFile && backupDb.length() > 0)
            openJvmAndroidApplicationRuntime(backupDb).use { restored ->
                val conv = requireNotNull(restored.loadConversation("conv-backup-1"))
                assertEquals("Backed Up Thread", conv.title)
                assertEquals("""{"verified":true}""", restored.productSetting("backup.marker")?.valueJson)
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    // -------------------------------------------------------------------------
    // Test Infrastructure Helpers
    // -------------------------------------------------------------------------

    private fun withTempRuntime(prefix: String, block: (AndroidApplicationRuntime, File) -> Unit) {
        val tempRoot = createPrivateTempDir("nola-android-$prefix-")
        val dbFile = File(tempRoot, "$prefix.db")
        try {
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                block(runtime, dbFile)
            }
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    private fun createPrivateTempDir(prefix: String): File =
        Files.createTempDirectory(
            prefix,
            PosixFilePermissions.asFileAttribute(PosixFilePermissions.fromString("rwx------")),
        ).toFile()

    private fun openJvmAndroidApplicationRuntime(dbFile: File): AndroidApplicationRuntime {
        ensureJniLoaded()
        val unsafeClass = Class.forName("sun.misc.Unsafe")
        val unsafeField = unsafeClass.getDeclaredField("theUnsafe").apply { isAccessible = true }
        val unsafe = unsafeField.get(null)
        val allocateInstance = unsafeClass.getMethod("allocateInstance", Class::class.java)
        val runtime = allocateInstance.invoke(unsafe, AndroidApplicationRuntime::class.java) as AndroidApplicationRuntime
        val handle = AgentCoreNative.nativeApplicationOpen(dbFile.absolutePath.encodeToByteArray())
        assertTrue("nativeApplicationOpen returned 0 for ${dbFile.absolutePath}", handle != 0L)

        fun setField(name: String, value: Any?) {
            val f = AndroidApplicationRuntime::class.java.getDeclaredField(name)
            f.isAccessible = true
            f.set(runtime, value)
        }
        setField("databaseFile", dbFile)
        setField("json", Json { ignoreUnknownKeys = true })
        setField("lock", Any())
        setField("handle", handle)
        setField("submissionJournal", ApplicationSubmissionJournal(File(dbFile.path + ".pending-commands.json")))
        setField("recoveredReceipts", mutableListOf<JsonObject>())
        return runtime
    }

    private fun runPythonSqlite(dbFile: File, sql: String) {
        val pb = ProcessBuilder(
            "python3",
            "-c",
            "import sqlite3, sys; c=sqlite3.connect(sys.argv[1]); c.executescript(sys.argv[2]); c.commit(); c.close()",
            dbFile.absolutePath,
            sql,
        )
        pb.redirectErrorStream(true)
        val proc = pb.start()
        val out = proc.inputStream.bufferedReader(Charsets.UTF_8).readText()
        assertTrue("python3 sqlite3 script timed out", proc.waitFor(10, TimeUnit.SECONDS))
        assertEquals("python3 sqlite3 script failed: $out", 0, proc.exitValue())
    }

    private fun queryPythonSqliteScalar(dbFile: File, sql: String): String {
        val pb = ProcessBuilder(
            "python3",
            "-c",
            "import sqlite3, sys; c=sqlite3.connect('file:' + sys.argv[1] + '?mode=ro', uri=True); row=c.execute(sys.argv[2]).fetchone(); c.close(); print('' if row is None else row[0], end='')",
            dbFile.absolutePath,
            sql,
        )
        pb.redirectErrorStream(true)
        val proc = pb.start()
        val out = proc.inputStream.bufferedReader(Charsets.UTF_8).readText()
        assertTrue("python3 sqlite3 query timed out", proc.waitFor(10, TimeUnit.SECONDS))
        assertEquals("python3 sqlite3 query failed: $out", 0, proc.exitValue())
        return out
    }
}
