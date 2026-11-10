package com.nola.data.local

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.*

/** Core-resolved view data. No A2UI parser, expression evaluator or data store here. */
@Serializable
data class UiSurfaceOwner(
    val authority_id: String,
    val conversation_id: String,
    val participant_id: String,
    val context_id: String,
    val originating_run_id: String,
    val message_id: String,
    val part_id: String,
)

@Serializable
data class UiSurfaceIdentity(val owner: UiSurfaceOwner, val surface_id: String, val generation: Long)

@Serializable
data class UiSurfaceRevisionRef(val identity: UiSurfaceIdentity, val revision: Long)

@Serializable
enum class UiSurfacePhase {
    @SerialName("pending") Pending,
    @SerialName("ready") Ready,
    @SerialName("read_only") ReadOnly,
    @SerialName("deleted") Deleted,
}

@Serializable
data class UiActionRef(val instance_id: String, val action_id: String)

@Serializable
data class UiNodeSnapshot(
    val instance_id: String,
    val component_id: String,
    val component: String,
    val props: Map<String, JsonElement>,
    val slots: Map<String, List<String>>,
    val bindings: Map<String, String>,
    val actions: Map<String, UiActionRef>,
    val validation_errors: List<String>,
)

@Serializable
data class UiSnapshot(
    val identity: UiSurfaceIdentity,
    val revision: Long,
    val catalog_id: String,
    val catalog_digest: String,
    val phase: UiSurfacePhase,
    val root_instance_id: String?,
    val nodes: Map<String, UiNodeSnapshot>,
    val summary: String,
)

@Serializable
data class UiChangeSet(
    val identity: UiSurfaceIdentity,
    val base_revision: Long,
    val revision: Long,
    val catalog_digest: String,
    val phase: UiSurfacePhase,
    val root_instance_id: String?,
    val upserted_nodes: List<UiNodeSnapshot>,
    val removed_instance_ids: List<String>,
    val summary: String,
)

@Serializable
sealed interface UiEdit {
    @Serializable @SerialName("set")
    data class Set(val instance_id: String, val binding: String, val value: JsonElement) : UiEdit
    @Serializable @SerialName("delete")
    data class Delete(val instance_id: String, val binding: String) : UiEdit
}

@Serializable
data class UiInteraction(
    val identity: UiSurfaceIdentity,
    val expected_surface_revision: Long,
    val edits: List<UiEdit>,
    val action: UiActionRef?,
)

/** Persist/retry the same instance; an absent acknowledgement is not rejection. */
@Serializable
data class UiSubmission(val interaction: UiInteraction, val submission_id: String, val submitted_at_ms: Long)

@Serializable
data class UiInteractionReceipt(val snapshot: UiSnapshot, val queued_submission_id: String?)

@Serializable
data class UiRemoteInteractionResult(
    val status: String,
    val receipt: UiInteractionReceipt? = null,
    val message: String? = null,
)

private val uiJson = Json { encodeDefaults = true; explicitNulls = true }

fun AndroidApplicationRuntime.listUiSurfaces(conversationId: String): List<UiSnapshot> {
    val result = executeQuery(buildJsonObject {
        put("type", "list_ui_surfaces")
        put("conversation_id", conversationId)
    })
    return uiJson.decodeFromString(result.toString())
}

fun AndroidApplicationRuntime.readUiSurfaceRevision(identity: UiSurfaceIdentity, revision: Long): UiSnapshot? {
    require(revision > 0)
    val result = executeQuery(buildJsonObject {
        put("type", "read_ui_surface_revision")
        put("identity", uiJson.parseToJsonElement(uiJson.encodeToString(identity)))
        put("revision", revision)
    })
    return uiJson.decodeFromString(result.toString())
}

fun AndroidApplicationRuntime.submitUiInteraction(input: UiSubmission): UiInteractionReceipt {
    val requestId = "ui-interaction:${input.interaction.identity.owner.conversation_id}:${input.submission_id}"
    val result = submitMutation(buildJsonObject {
        put("type", "submit_ui_interaction")
        put("input", uiJson.parseToJsonElement(uiJson.encodeToString(input)))
    }, requestId)
    return uiJson.decodeFromString(result.toString())
}

@Serializable
sealed interface UiSurfaceUpdate {
    @Serializable @SerialName("snapshot") data class Snapshot(val snapshot: UiSnapshot) : UiSurfaceUpdate
    @Serializable @SerialName("changes") data class Changes(val changes: UiChangeSet) : UiSurfaceUpdate
    @Serializable @SerialName("unchanged") data class Unchanged(val identity: UiSurfaceIdentity, val revision: Long) : UiSurfaceUpdate
    @Serializable @SerialName("missing") data class Missing(val identity: UiSurfaceIdentity) : UiSurfaceUpdate
}

fun applyUiSurfaceUpdate(identity: UiSurfaceIdentity, baseline: UiSnapshot?, update: UiSurfaceUpdate): UiSnapshot? {
    val owner = when (update) {
        is UiSurfaceUpdate.Snapshot -> update.snapshot.identity
        is UiSurfaceUpdate.Changes -> update.changes.identity
        is UiSurfaceUpdate.Unchanged -> update.identity
        is UiSurfaceUpdate.Missing -> update.identity
    }
    require(owner == identity && (baseline == null || baseline.identity == identity)) { "UI update owner mismatch" }
    return when (update) {
        is UiSurfaceUpdate.Missing -> null
        is UiSurfaceUpdate.Snapshot -> update.snapshot.also { require(baseline == null || it.revision >= baseline.revision) }
        is UiSurfaceUpdate.Unchanged -> checkNotNull(baseline).also { require(it.revision == update.revision) }
        is UiSurfaceUpdate.Changes -> {
            val previous = checkNotNull(baseline)
            val change = update.changes
            require(previous.revision == change.base_revision && previous.catalog_digest == change.catalog_digest && change.revision > change.base_revision)
            val nodes = previous.nodes.toMutableMap()
            change.removed_instance_ids.forEach { nodes.remove(it) }
            change.upserted_nodes.forEach { nodes[it.instance_id] = it }
            require(change.root_instance_id == null || nodes.containsKey(change.root_instance_id))
            previous.copy(revision = change.revision, phase = change.phase, root_instance_id = change.root_instance_id, nodes = nodes, summary = change.summary)
        }
    }
}

fun AndroidApplicationRuntime.readUiSurfaceChanges(identity: UiSurfaceIdentity, baseRevision: Long?): UiSurfaceUpdate {
    val result = executeQuery(buildJsonObject {
        put("type", "read_ui_surface_changes")
        put("identity", uiJson.parseToJsonElement(uiJson.encodeToString(identity)))
        put("base_revision", baseRevision?.let(::JsonPrimitive) ?: JsonNull)
    })
    return uiJson.decodeFromString(result.toString())
}

/** Historical wrappers provide only an exact read-only anchor, never active UI authority. */
data class UiReferencePresentation(val partId: String, val reference: UiSurfaceRevisionRef, val summary: String, val historical: Boolean)
fun uiReferencePresentation(part: com.nola.domain.projection.ConversationProjectionV1.Part, conversationId: String?, messageId: String): UiReferencePresentation? {
    val reference: UiSurfaceRevisionRef
    val summary: String
    val historical: Boolean
    when (part) {
        is com.nola.domain.projection.ConversationProjectionV1.Part.UiSurface -> { reference = part.surface; summary = part.summary; historical = false }
        is com.nola.domain.projection.ConversationProjectionV1.Part.Historical -> {
            val projection = part.projection
            if (projection.schema != "message_part_view" || projection.canonicalKind != "ui_surface" || projection.binding != "canonical" ||
                projection.status != com.nola.domain.projection.ConversationProjectionV1.HistoricalStatus.Supported) return null
            val fields = projection.fields
            val raw = (fields["surface"] as? com.nola.domain.projection.ConversationProjectionV1.HistoricalField.Complete)?.value ?: return null
            val label = (fields["summary"] as? com.nola.domain.projection.ConversationProjectionV1.HistoricalField.Complete)?.value as? JsonPrimitive ?: return null
            require(projection.owner["conversation_id"]?.jsonPrimitive?.content == conversationId &&
                projection.owner["message_id"]?.jsonPrimitive?.content == messageId && projection.owner["part_id"]?.jsonPrimitive?.content == part.partId)
            reference = uiJson.decodeFromString(raw.toString()); summary = label.content; historical = true
        }
        else -> return null
    }
    require(reference.revision > 0 && reference.identity.owner.conversation_id == conversationId && reference.identity.owner.message_id == messageId)
    return UiReferencePresentation(part.partId, reference, summary, historical)
}

@Serializable
data class UiPreviewInput(val identity: UiSurfaceIdentity, val expected_surface_revision: Long,
    val edits: List<UiEdit>, val draft_generation: Long)

@Serializable
data class UiPreview(val identity: UiSurfaceIdentity, val base_revision: Long,
    val draft_generation: Long, val snapshot: UiSnapshot)

fun UiPreview.requireMatches(input: UiPreviewInput, baseline: UiSnapshot): UiSnapshot {
    require(identity == input.identity && identity == baseline.identity && base_revision == input.expected_surface_revision &&
        base_revision == baseline.revision && draft_generation == input.draft_generation && snapshot.identity == identity &&
        snapshot.revision == base_revision && snapshot.phase == UiSurfacePhase.Ready && snapshot.catalog_digest == baseline.catalog_digest && snapshot.catalog_id == baseline.catalog_id)
    return snapshot
}

fun AndroidApplicationRuntime.previewUiSurface(input: UiPreviewInput): UiPreview = uiJson.decodeFromString(executeQuery(buildJsonObject {
    put("type", "preview_ui_surface")
    put("input", uiJson.parseToJsonElement(uiJson.encodeToString(input)))
}).toString())
