package com.nola.ui.screen.chat

import android.database.sqlite.SQLiteDatabase
import com.nola.data.local.*
import java.security.MessageDigest
import kotlinx.serialization.json.*

internal data class RealExpertTemplateFixture(val id: String, val name: String, val body: String,
    val owner: String, val catalogRevision: Long) {
    val digest: String get() = sha(body)
    companion object {
        fun sha(text: String): String = MessageDigest.getInstance("SHA-256").digest(text.encodeToByteArray())
            .joinToString("") { "%02x".format(it) }
    }
}

/** User-owned fixture update through current checked Core CAS, with all other
 * records retained byte-for-value. No old capture fact or message is changed. */
internal fun updateRealExpertTemplateFixture(runtime: AndroidApplicationRuntime, fixture: RealExpertTemplateFixture): Long =
    updateRealTemplateCatalog(runtime) { templates -> templates.map { item ->
        val record = item.jsonObject
        if (record["id"] != JsonPrimitive(fixture.id)) item else {
            check(record["content"] == JsonPrimitive(fixture.body) && record["accessOwnerID"] == JsonPrimitive(fixture.owner))
            JsonObject(record + mapOf("content" to JsonPrimitive("Use this later definition for future messages only."),
                "accessRevision" to JsonPrimitive(2), "updatedAt" to JsonPrimitive(System.currentTimeMillis() / 1000.0 - 978307200.0)))
        }
    } }

private fun updateRealTemplateCatalog(runtime: AndroidApplicationRuntime, update: (List<JsonElement>) -> List<JsonElement>): Long {
    val before = runtime.productSetting("prompt.templates")
    val catalog = before?.valueJson?.let { Json.parseToJsonElement(it).jsonObject } ?: buildJsonObject { put("customTemplates", JsonArray(emptyList())) }
    val existing = catalog.getValue("customTemplates").jsonArray.toList()
    val next = JsonObject(catalog + ("customTemplates" to JsonArray(update(existing))))
    val result = runtime.execute(buildJsonObject {
        put("type", "put_product_setting_checked"); put("key", "prompt.templates"); put("value_json", next.toString())
        put("expected_revision", before?.revision?.let(::JsonPrimitive) ?: JsonNull); put("updated_at_ms", System.currentTimeMillis())
    }).jsonObject
    return result.getValue("revision").jsonPrimitive.long
}

/** Test-only source-Store witness inspection. Production never opens this
 * private table or obtains capture authority from a renderer setting lookup. */
internal fun assertRealExpertTemplateCapture(runtime: AndroidApplicationRuntime, manifest: AndroidExpertMessageManifest,
    fixture: RealExpertTemplateFixture) {
    val entry = manifest.entries.single()
    val expected = checkNotNull(entry.expectedPromptDefinition)
    check(expected.componentId == "prompt_template:${fixture.id}" && expected.content == fixture.body && expected.contentSha256 == fixture.digest)
    val actual = checkNotNull(entry.instructionWitness).promptDefinitions.single { it.componentId == "prompt_template:${fixture.id}" }
    check(actual.content == fixture.body && actual.contentSha256 == fixture.digest && actual.definitionRevision == expected.definitionRevision)
    val owner = JsonArray(listOf(JsonPrimitive(manifest.input.conversationId), JsonPrimitive(manifest.input.submissionId))).toString()
    val key = "edited_resend/expert/template_capture/" + RealExpertTemplateFixture.sha(owner)
    SQLiteDatabase.openDatabase(runtime.databaseFile.path, null,
        SQLiteDatabase.OPEN_READONLY or SQLiteDatabase.NO_LOCALIZED_COLLATORS).use { db ->
        db.rawQuery("SELECT value_json FROM product_settings WHERE key=?", arrayOf(key)).use { cursor ->
            check(cursor.moveToFirst()) { "The actual native admission has no durable capture fact" }
            val fact = Json.parseToJsonElement(cursor.getString(0)).jsonObject
            check(!cursor.moveToNext())
            check(fact.getValue("conversation_id") == JsonPrimitive(manifest.input.conversationId) &&
                fact.getValue("submission_id") == JsonPrimitive(manifest.input.submissionId) &&
                fact.getValue("actor").jsonObject.getValue("id") == JsonPrimitive(fixture.owner))
            val captured = fact.getValue("entries").jsonArray.single().jsonObject
            check(captured.getValue("participant_id") == JsonPrimitive(entry.participant.participantId) &&
                captured.getValue("context_conversation_id") == JsonPrimitive(entry.participant.contextConversationId) &&
                captured.getValue("child_submission_id") == JsonPrimitive(entry.childSubmissionId) &&
                captured.getValue("definition_sha256") == JsonPrimitive(fixture.digest))
            val origin = captured.getValue("origin").jsonObject
            check(origin.getValue("kind") == JsonPrimitive("custom") &&
                origin.getValue("catalog_revision") == JsonPrimitive(fixture.catalogRevision) &&
                origin.getValue("resource").jsonObject.getValue("resource_id") == JsonPrimitive(fixture.id) &&
                origin.getValue("decision").jsonObject.getValue("allowed") == JsonPrimitive(true))
        }
    }
}
