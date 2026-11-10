package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Test
import org.junit.Rule
import org.junit.rules.TemporaryFolder
import kotlin.test.*

class UiInteractionIntentStoreTest {
    @get:Rule val directory = TemporaryFolder()
    @Test fun exactRequestSurvivesRestartAndCannotMoveToAnotherHost() {
        val file = java.io.File(directory.root, "ui-intents.json")
        val identity = UiSurfaceIdentity(UiSurfaceOwner("authority", "conversation", "agent", "context", "run", "message", "tool"), "form", 1)
        val input = UiSubmission(UiInteraction(identity, 1, emptyList(), UiActionRef("button", "action")), "submission", 123)
        val original = UiInteractionIntentStore(file)
        val source = buildJsonObject { put("host_id", "host"); put("base_url", "https://host") }
        val request = original.stage(input, source)
        val reopened = UiInteractionIntentStore(file)
        assertEquals(input, reopened.pending(identity))
        assertEquals(request, reopened.stage(input, source))
        assertFails { reopened.stage(input, buildJsonObject { put("host_id", "another") }) }
        reopened.retire(request)
        assertNull(UiInteractionIntentStore(file).pending(identity))
    }
}
