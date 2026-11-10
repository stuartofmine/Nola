package com.nola.data.repository
import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.projection.ConversationProjectionV1Decoder
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
class TaskControlCapabilitiesTest {
    private fun task(kind: String, guidance: Boolean? = null): JsonObject = buildJsonObject {
        put("kind",kind); put("part_id","part"); put("revision",1)
        put("task_id","task"); put("display_key","agent"); put("state","running")
        put("progress",JsonNull); put("current_step",JsonNull); put("result_ref",JsonNull)
        put("cancel_requested",false)
        guidance?.let { put("supports_guidance",it); put("supports_cancellation",true) }
    }
    private fun decode(value: JsonObject) = ConversationProjectionV1Decoder.decodeDisplayPart(value,"message") as ConversationProjectionV1.Part.Task
    @Test fun plainTaskNeverInfersAuthorityFromRunningState() {
        val value=decode(task("task")); assertFalse(value.supportsGuidance); assertFalse(value.supportsCancellation)
    }
    @Test fun controlRequiresCoreCapabilityAndPreservesFalse() {
        assertThrows(RuntimeException::class.java) { decode(task("task_control")) }
        assertFalse(decode(task("task_control",false)).supportsGuidance)
        val value=decode(task("task_control",true)); assertTrue(value.supportsGuidance); assertTrue(value.supportsCancellation)
    }
}
