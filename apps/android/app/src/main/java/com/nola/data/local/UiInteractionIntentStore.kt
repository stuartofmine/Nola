package com.nola.data.local

import java.io.File
import kotlinx.serialization.encodeToString
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.*

/** Reuses the fsynced, locked application intent journal; never stores Host tokens. */
internal class UiInteractionIntentStore(file: File) {
    private val journal = ApplicationSubmissionJournal(file)
    fun stage(input: UiSubmission, source: JsonObject): JsonObject = journal.prepare(buildJsonObject {
        put("type", "submit_ui_interaction")
        put("input", Json.parseToJsonElement(Json.encodeToString(input)))
        put("source", source)
    }, 1, "ui-interaction:${input.interaction.identity.owner.conversation_id}:${input.submission_id}", null)
    fun pending(identity: UiSurfaceIdentity): UiSubmission? = journal.pending().map { envelope ->
        val arguments = envelope.getValue("command").jsonObject.getValue("arguments").jsonObject
        Json.decodeFromString<UiSubmission>(arguments.getValue("input").toString())
    }.filter { it.interaction.identity == identity }.also { require(it.size <= 1) }.singleOrNull()
    fun retire(envelope: JsonObject) = journal.retireUiIntent(envelope)
}
