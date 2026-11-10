package com.nola.domain.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class ModelInputAttachmentTest {
    @Test
    fun nativeImageIsInjectedIntoLastUserMessageOnly() {
        val messages = listOf(
            message("user", "earlier"),
            message("assistant", "answer"),
            message("user", "analyze this"),
        )
        val rendered = ModelInput(
            canonicalMessages = messages,
            attachments = listOf(ModelAttachment("chart.png", "image/png", "AQID")),
        ).providerMessages()

        assertEquals(JsonPrimitive("earlier"), rendered[0]["content"])
        val content = rendered[2]["content"] as JsonArray
        assertEquals("text", (content[0] as JsonObject)["type"]?.jsonPrimitiveContent())
        assertEquals("analyze this", (content[0] as JsonObject)["text"]?.jsonPrimitiveContent())
        val image = content[1] as JsonObject
        assertEquals("image_url", image["type"]?.jsonPrimitiveContent())
        assertTrue(image["image_url"].toString().contains("data:image/png;base64,AQID"))
    }

    private fun message(role: String, content: String) = JsonObject(
        mapOf("role" to JsonPrimitive(role), "content" to JsonPrimitive(content)),
    )

    private fun kotlinx.serialization.json.JsonElement.jsonPrimitiveContent(): String =
        (this as JsonPrimitive).content
}
