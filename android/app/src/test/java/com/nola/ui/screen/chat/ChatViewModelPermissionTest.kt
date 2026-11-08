package com.nola.ui.screen.chat

import com.nola.R
import com.nola.core.AgentCoreCapabilityActivationEnvelope
import com.nola.domain.model.HostAgentPermissionRequest
import com.nola.domain.model.HostAgentToolEvent
import com.nola.domain.model.decodeHostToolEventsFromMeta
import com.nola.domain.model.encodeHostToolEventsForMeta
import com.nola.ui.components.hostToolIsBrowserTool
import com.nola.ui.components.hostToolIsComputerTool
import com.nola.ui.components.hostToolShouldShowComputerScreenshotHint
import com.nola.ui.components.hostToolStatusStringRes
import com.nola.ui.components.openableToolUri
import com.nola.util.hostToolContainsSensitiveData
import com.nola.util.redactHostToolText
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatViewModelPermissionTest {
    @Test fun savedPermissionUsesValidatedSourceStateWithoutReconstructingItsRevision() {
        val saved = ChatUiState(conversationId = "conversation", processingState = com.nola.domain.model.ProcessingState.Streaming,
            permissionMode = com.nola.core.AndroidAgentPermissionMode.AllowAll, permissionPolicyRunId = "run", permissionPolicyRevision = 12,
            permissionConfigurationOwner = com.nola.data.local.PermissionConfigurationOwner.Remote(
                com.nola.data.remote.api.RemoteSubmissionIdentity("host", "client", "conversation", "run", "user", "assistant")))
        assertEquals("checking", saved.permissionApplicationStatus)
        assertEquals("checking", saved.copy(remotePermissionPolicy = com.nola.core.AgentActivePermissionPolicy("run", 11, "allow_all")).permissionApplicationStatus)
        assertEquals("checking", saved.copy(remotePermissionPolicy = com.nola.core.AgentActivePermissionPolicy("foreign", 12, "allow_all")).permissionApplicationStatus)
        assertEquals("checking", saved.copy(remotePermissionPolicy = com.nola.core.AgentActivePermissionPolicy("run", 12, "allow_all")).permissionApplicationStatus)
        assertEquals("pending", saved.copy(remotePermissionApplicationState = "pending").permissionApplicationStatus)
        assertEquals("active", saved.copy(remotePermissionApplicationState = "active").permissionApplicationStatus)
        assertEquals("failed", saved.copy(permissionSaveFailed = true).permissionApplicationStatus)
        for (state in listOf(com.nola.domain.model.ProcessingState.Idle, com.nola.domain.model.ProcessingState.Error)) {
            assertEquals("checking", saved.copy(processingState = state).permissionApplicationStatus)
            assertEquals("active", saved.copy(processingState = state, remotePermissionApplicationState = "active",
                remotePermissionPolicy = com.nola.core.AgentActivePermissionPolicy("run", 12, "allow_all")).permissionApplicationStatus)
        }
        assertEquals("saved", saved.copy(permissionConfigurationOwner = com.nola.data.local.PermissionConfigurationOwner.Inactive).permissionApplicationStatus)
        assertEquals("checking", saved.copy(permissionConfigurationOwner = null, processingState = com.nola.domain.model.ProcessingState.Idle).permissionApplicationStatus)
    }

    @Test
    fun permissionResumeAbortsWhenConversationIsUnavailable() {
        assertTrue(shouldAbortHostPermissionResume(null, conversationExists = true))
        assertTrue(shouldAbortHostPermissionResume("", conversationExists = true))
        assertTrue(shouldAbortHostPermissionResume("conversation-1", conversationExists = false))
        assertFalse(shouldAbortHostPermissionResume("conversation-1", conversationExists = true))
    }

    @Test
    fun permissionResumeConversationIdRequiresNonBlankId() {
        assertNull(hostPermissionResumeConversationId(null))
        assertNull(hostPermissionResumeConversationId(""))
        assertNull(hostPermissionResumeConversationId("   "))
        assertEquals("conversation-1", hostPermissionResumeConversationId("conversation-1"))
    }

    @Test
    fun permissionDecisionPayloadSupportsAllowAndDenyResume() {
        val request = HostAgentPermissionRequest(
            allowConversation = true,
            id = "call-1",
            toolName = "computer_click",
            match = "click:100,200"
        )

        val allow = hostPermissionDecisionPayload(request, allow = true)
        val deny = hostPermissionDecisionPayload(request, allow = false)
        val conversation = hostPermissionDecisionPayload(request, allow = true, rememberForConversation = true)

        val allowResolution = allow["resolution"] as Map<*, *>
        val denyResolution = deny["resolution"] as Map<*, *>
        val conversationResolution = conversation["resolution"] as Map<*, *>
        assertEquals("authorization", allowResolution["type"])
        assertEquals("allow_once", allowResolution["decision"])
        assertEquals("deny", denyResolution["decision"])
        assertEquals("call-1", deny["id"])
        assertEquals("computer_click", deny["tool_name"])
        assertEquals("click:100,200", deny["match"])
        assertEquals("allow_conversation", conversationResolution["decision"])
        assertFalse(conversation.containsKey("grant_scope"))
    }

    @Test
    fun localPermissionChoiceUsesCoreConversationDecision() {
        val once = com.nola.core.AndroidAgentPermissionDecision(
            requestId = "permission-1",
            decision = com.nola.core.AndroidAgentPermissionUserDecision.AllowOnce,
        )

        assertEquals("allow_once", com.nola.core.corePermissionDecisionWireValue(once))
        assertEquals(
            "allow_conversation",
            com.nola.core.corePermissionDecisionWireValue(
                once.copy(decision = com.nola.core.AndroidAgentPermissionUserDecision.AllowConversation)
            ),
        )
        assertEquals(
            "deny",
            com.nola.core.corePermissionDecisionWireValue(
                once.copy(decision = com.nola.core.AndroidAgentPermissionUserDecision.Deny)
            ),
        )
    }

    @Test
    fun permissionRequiredToolResultUsesPermissionStatusInsteadOfError() {
        val stringMeta = mapOf<String, Any>("permission_required" to "true")
        val boolMeta = mapOf<String, Any>("permission_required" to true)
        val numericMeta = mapOf<String, Any>("permission_required" to 1)
        val invalidNumericMeta = mapOf<String, Any>("permission_required" to 2)

        assertTrue(hostToolMetaFlag(stringMeta, "permission_required"))
        assertTrue(hostToolMetaFlag(boolMeta, "permission_required"))
        assertTrue(hostToolMetaFlag(numericMeta, "permission_required"))
        assertFalse(hostToolMetaFlag(invalidNumericMeta, "permission_required"))
        assertEquals("export.csv", hostToolMetaText(mapOf("download_filename" to " export.csv "), "download_filename"))
        assertEquals("permission_required", hostToolResultStatus(eventError = true, meta = stringMeta))
        assertFalse(hostToolResultError(eventError = true, meta = stringMeta))
        assertEquals("error", hostToolResultStatus(eventError = true, meta = invalidNumericMeta))
        assertTrue(hostToolResultError(eventError = true, meta = invalidNumericMeta))
        assertEquals("error", hostToolResultStatus(eventError = true, meta = emptyMap()))
        assertTrue(hostToolResultError(eventError = true, meta = emptyMap()))
        assertEquals(
            R.string.chat_host_tool_status_permission_required,
            hostToolStatusStringRes("permission_required", error = true)
        )
    }

    @Test
    fun permissionResumeLimitUsesConfiguredMaximum() {
        assertTrue(canResumeHostPermission(permissionResumeCount = 0, maxPermissionResumes = 1))
        assertFalse(canResumeHostPermission(permissionResumeCount = 1, maxPermissionResumes = 1))
        assertTrue(canResumeHostPermission(permissionResumeCount = 2, maxPermissionResumes = 30))
        assertFalse(canResumeHostPermission(permissionResumeCount = 30, maxPermissionResumes = 30))
        assertTrue(canResumeHostPermission(permissionResumeCount = 0, maxPermissionResumes = 0))
        assertTrue(canResumeHostPermission(permissionResumeCount = 10_000, maxPermissionResumes = 0))
    }

    @Test
    fun upsertHostToolEventPreservesTimelineWithoutDuplicates() {
        val first = HostAgentToolEvent(id = "call-1", toolName = "computer_click", status = "running")
        val second = HostAgentToolEvent(id = "call-2", toolName = "computer_screenshot", status = "running")
        val updatedFirst = first.copy(status = "done", resultSummary = "clicked")

        val initial = upsertHostToolEvent(emptyList(), first)
        val appended = upsertHostToolEvent(initial, second)
        val updated = upsertHostToolEvent(appended, updatedFirst)

        assertEquals(2, updated.size)
        assertEquals("call-1", updated[0].id)
        assertEquals("done", updated[0].status)
        assertEquals("clicked", updated[0].resultSummary)
        assertEquals("call-2", updated[1].id)
    }

    @Test
    fun capabilityActivationBufferRetainsLatestEventAcrossConversationSwitches() {
        val buffer = CapabilityActivationBuffer()
        val first = AgentCoreCapabilityActivationEnvelope(
            sessionId = "conversation-1",
            turnId = "turn-1",
            cursor = 7,
            journalRevision = 7,
            source = "skill-runtime",
            capabilityIds = listOf("documents.create"),
        )
        val stale = first.copy(cursor = 6, capabilityIds = listOf("stale.capability"))
        val latest = first.copy(cursor = 8, capabilityIds = listOf("documents.create", "files.write"))

        buffer.stage(first)
        buffer.stage(stale)
        assertEquals("documents.create", buffer.events("conversation-1").single().resultSummary)

        buffer.stage(latest)
        val restored = buffer.events("conversation-1")
        assertEquals(1, restored.size)
        assertEquals("documents.create\nfiles.write", restored.single().resultSummary)
        assertTrue(buffer.events("conversation-2").isEmpty())

        buffer.clear("conversation-1")
        assertTrue(buffer.events("conversation-1").isEmpty())
    }

    @Test
    fun typedToolCardKeepsParametersAndRealResultIndependentOfAuditStatus() {
        val part = com.nola.domain.projection.ConversationProjectionV1Decoder.decodeDisplayPart(
            kotlinx.serialization.json.Json.parseToJsonElement("""{
                "kind":"tool","execution_source":{"kind":"model"},"part_id":"tool-1","revision":1,"tool_call_id":"call","capability_id":"read_file",
                "title":"Read file","state":"completed","input":{"path":"/tmp/report.md"},"origin":{"kind":"builtin"},
                "output_preview":"42 lines read","affected_resources":[],"artifacts":[]
            }""") as JsonObject, "assistant") as com.nola.domain.projection.ConversationProjectionV1.Part.Tool
        assertTrue(part.input.toString().contains("/tmp/report.md"))
        assertEquals(null, part.summary)
        assertEquals(null, part.output)
        assertEquals("42 lines read", part.outputPreview)
        assertFalse(part.summary.orEmpty().contains("tool_completed"))
    }

    @Test
    fun hostToolEventsRoundTripThroughMessageMeta() {
        val events = listOf(
            HostAgentToolEvent(
                id = "call-1",
                toolName = "computer_screenshot",
                status = "done",
                resultSummary = "captured",
                screenshotUrl = "/static/agent_artifacts/computer/call-1.png",
                pageTitle = "Desktop",
                downloadUrl = "http://127.0.0.1/static/agent_artifacts/browser/user/downloads/export.csv",
                downloadFilename = "export.csv",
                screenshotWarning = "native Chrome screenshot is too large"
            ),
            HostAgentToolEvent(
                id = "call-2",
                toolName = "computer_click",
                status = "error",
                resultSummary = "blocked",
                error = true
            )
        )

        val meta = encodeHostToolEventsForMeta(events)
        val decoded = decodeHostToolEventsFromMeta(meta)

        assertEquals(2, decoded.size)
        assertEquals("computer_screenshot", decoded[0].toolName)
        assertEquals("/static/agent_artifacts/computer/call-1.png", decoded[0].screenshotUrl)
        assertEquals("Desktop", decoded[0].pageTitle)
        assertEquals("http://127.0.0.1/static/agent_artifacts/browser/user/downloads/export.csv", decoded[0].downloadUrl)
        assertEquals("export.csv", decoded[0].downloadFilename)
        assertEquals("native Chrome screenshot is too large", decoded[0].screenshotWarning)
        assertEquals("error", decoded[1].status)
        assertTrue(decoded[1].error)
    }

    @Test
    fun hostToolEventsMetaRedactsSensitivePersistedEvents() {
        val events = listOf(
            HostAgentToolEvent(
                id = "call-secret",
                toolName = "chrome_native_open_page",
                status = "done",
                resultSummary = "Opened callback access_token=secret-value",
                screenshotUrl = "https://host.local/static/call.png",
                finalUrl = "https://example.com/callback?access_token=secret-value",
                pageTitle = "Callback access_token=secret-value",
                downloadUrl = "https://host.local/static/export.csv?access_token=secret-value",
                downloadFilename = "export-access_token=secret-value.csv",
                screenshotWarning = "Screenshot failed token=secret-value"
            )
        )

        val decoded = decodeHostToolEventsFromMeta(encodeHostToolEventsForMeta(events))

        assertEquals(1, decoded.size)
        assertNull(decoded[0].resultSummary)
        assertNull(decoded[0].screenshotUrl)
        assertNull(decoded[0].finalUrl)
        assertNull(decoded[0].pageTitle)
        assertNull(decoded[0].downloadUrl)
        assertNull(decoded[0].downloadFilename)
        assertEquals("Screenshot failed token=[hidden]", decoded[0].screenshotWarning)
        assertTrue(decoded[0].sensitive)
    }

    @Test
    fun finalAssistantResponseUsesEmptyFallbackOnlyForToolOnlyRuns() {
        assertEquals(
            "Done",
            finalAssistantResponseText("  Done\n", hasToolEvents = true, emptyResponseText = "(No content returned)")
        )
        assertEquals(
            "(No content returned)",
            finalAssistantResponseText("   ", hasToolEvents = true, emptyResponseText = "(No content returned)")
        )
        assertEquals(
            "",
            finalAssistantResponseText("   ", hasToolEvents = false, emptyResponseText = "(No content returned)")
        )
    }

    @Test
    fun hostToolCardOnlyOpensHttpUris() {
        assertEquals("https://host.local/static/call.png", openableToolUri(" https://host.local/static/call.png "))
        assertEquals("http://127.0.0.1:5000/static/call.png", openableToolUri("http://127.0.0.1:5000/static/call.png"))
        assertEquals("HTTPS://host.local/static/call.png", openableToolUri("HTTPS://host.local/static/call.png"))
        assertNull(openableToolUri("/static/agent_artifacts/call.png"))
        assertNull(openableToolUri("//host.local/static/call.png"))
        assertNull(openableToolUri("file:///Users/me/secret.png"))
        assertNull(openableToolUri("https://user:pass@host.local/static/call.png"))
        assertNull(openableToolUri("https://host.local/static/call\n.png"))
        assertNull(openableToolUri("https://"))
        assertNull(openableToolUri(""))
    }

    @Test
    fun hostToolCardClassifiesBrowserAndComputerToolsLikeIos() {
        assertTrue(hostToolIsBrowserTool("chrome_native_open_page"))
        assertTrue(hostToolIsBrowserTool("chrome_native_screenshot"))
        assertFalse(hostToolIsBrowserTool("computer_screenshot"))

        assertTrue(hostToolIsComputerTool("computer_screenshot"))
        assertFalse(hostToolIsComputerTool("chrome_native_screenshot"))

        assertTrue(
            hostToolShouldShowComputerScreenshotHint(
                toolName = "computer_screenshot",
                containsSensitiveData = false,
                screenshotUri = "http://10.0.2.2:5001/static/call.png"
            )
        )
        assertFalse(
            hostToolShouldShowComputerScreenshotHint(
                toolName = "chrome_native_screenshot",
                containsSensitiveData = false,
                screenshotUri = "http://10.0.2.2:5001/static/call.png"
            )
        )
        assertFalse(
            hostToolShouldShowComputerScreenshotHint(
                toolName = "computer_screenshot",
                containsSensitiveData = true,
                screenshotUri = "http://10.0.2.2:5001/static/call.png"
            )
        )
    }

    @Test
    fun hostToolResultSummaryFallsBackToBrowserTitleAndDownloadName() {
        assertEquals(
            "Opened",
            hostToolResultSummary(
                display = " Opened ",
                text = "raw text",
                pageTitle = "Page",
                downloadFilename = "export.csv",
                sensitive = false
            )
        )
        assertEquals(
            "Example Domain",
            hostToolResultSummary(
                display = " ",
                text = "",
                pageTitle = " Example Domain ",
                downloadFilename = null,
                sensitive = false
            )
        )
        assertEquals(
            "export.csv",
            hostToolResultSummary(
                display = null,
                text = " ",
                pageTitle = null,
                downloadFilename = " export.csv ",
                sensitive = false
            )
        )
        assertEquals(
            "raw fallback",
            hostToolResultSummary(
                display = null,
                text = " \n raw fallback",
                pageTitle = null,
                downloadFilename = null,
                sensitive = false
            )
        )
        assertNull(
            hostToolResultSummary(
                display = "Opened token=secret-value",
                text = "raw",
                pageTitle = "Page",
                downloadFilename = null,
                sensitive = true
            )
        )
    }

    @Test
    fun hostToolSensitiveTextIsDetectedAndRedacted() {
        val summary = "Saved token=secret-value Authorization: Bearer abcdefghijk12345"
        val jsonSummary = """{"token":"json-secret-value","secret":"json-second-secret","session":"json-session-secret"}"""
        val url = "https://example.com/callback?access_token=secret-value"

        assertTrue(hostToolContainsSensitiveData(summary, jsonSummary, url))
        assertEquals("Saved token=[hidden] Authorization: [hidden]", redactHostToolText(summary))
        assertFalse(redactHostToolText(summary).contains("secret-value"))
        assertFalse(redactHostToolText(summary).contains("abcdefghijk12345"))
        assertFalse(redactHostToolText(jsonSummary).contains("json-secret-value"))
        assertFalse(redactHostToolText(jsonSummary).contains("json-second-secret"))
        assertFalse(redactHostToolText(jsonSummary).contains("json-session-secret"))
    }

    @Test
    fun hostToolStatusLabelsUseLocalizedResources() {
        assertEquals(R.string.chat_host_tool_status_running, hostToolStatusStringRes("running", error = false))
        assertEquals(R.string.chat_host_tool_status_succeeded, hostToolStatusStringRes("done", error = false))
        assertEquals(R.string.chat_host_tool_status_succeeded, hostToolStatusStringRes("succeeded", error = false))
        assertEquals(R.string.chat_host_tool_status_failed, hostToolStatusStringRes("done", error = true))
        assertEquals(R.string.chat_host_tool_status_failed, hostToolStatusStringRes("failed", error = false))
        assertEquals(
            R.string.chat_host_tool_status_permission_required,
            hostToolStatusStringRes("permission_required", error = false)
        )
        assertNull(hostToolStatusStringRes("custom_state", error = false))
    }
}
