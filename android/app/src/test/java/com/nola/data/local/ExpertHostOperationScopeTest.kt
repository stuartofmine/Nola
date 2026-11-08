package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class ExpertHostOperationScopeTest {
    private fun admit(parent: String, vararg children: String) = buildJsonObject {
        put("type", "native_expert_host_operation")
        put("operation", buildJsonObject {
            put("kind", "admit"); put("input", buildJsonObject {
                put("conversation_id", parent)
                put("selected_participants", JsonArray(children.map { buildJsonObject { put("context_conversation_id", it) } }))
            })
        })
    }
    @Test fun completeParentAndChildOwnershipSerializesOnlyOverlappingIntents() {
        val a = expertHostOperationScope(admit("parent-a", "child-a", "shared"))
        assertTrue(a.conflicts(ApplicationCommandScope.Conversation("parent-a")))
        assertTrue(a.conflicts(expertHostOperationScope(admit("parent-b", "shared"))))
        assertFalse(a.conflicts(expertHostOperationScope(admit("parent-c", "child-c"))))
        assertTrue(ApplicationCommandScope.Global.conflicts(a))
    }
    @Test fun editedIntentOwnsSourceAndTargetButDoesNotGuessFutureChildren() {
        val scope = expertHostOperationScope(buildJsonObject {
            put("type", "native_expert_host_operation"); put("operation", buildJsonObject {
                put("kind", "prepare_edited_resend"); put("input", buildJsonObject {
                    put("source_conversation_id", "source"); put("target_conversation_id", "target")
                })
            })
        })
        assertEquals(ApplicationCommandScope.Owners(setOf("source", "target")), scope)
        assertFalse(scope.conflicts(ApplicationCommandScope.Conversation("unrelated")))
    }
    @Test fun unknownOrIncompletePrivateShapesStayGlobalAndRendererClassifierIsUnchanged() {
        assertEquals(ApplicationCommandScope.Global, expertHostOperationScope(admit("parent")))
        assertEquals(ApplicationCommandScope.Global, expertHostOperationScope(buildJsonObject { put("type", "admit") }))
        val native = admit("parent", "child")
        assertEquals(ApplicationCommandScope.Global, ApplicationCommandScope.command(native))
    }
}
