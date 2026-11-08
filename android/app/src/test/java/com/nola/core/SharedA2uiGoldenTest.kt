package com.nola.core

import com.nola.data.local.*
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class SharedA2uiGoldenTest {
    @Test fun draftPreviewRequiresExactOwnerRevisionCatalogAndGeneration() {
        val fixture = Json.parseToJsonElement(checkNotNull(javaClass.classLoader?.getResourceAsStream("a2ui/form_interaction_v0_9_1.json"))
            .bufferedReader().use { it.readText() }).jsonObject
        val initial = Json.decodeFromJsonElement(UiSnapshot.serializer(),fixture.getValue("initial_snapshot"))
        val input = UiPreviewInput(initial.identity,initial.revision,emptyList(),3)
        val valid = UiPreview(initial.identity,initial.revision,3,initial)
        assertEquals(initial,valid.requireMatches(input,initial))
        for (bad in listOf(valid.copy(draft_generation=2),valid.copy(base_revision=initial.revision+1),
            valid.copy(identity=initial.identity.copy(generation=2)),
            valid.copy(snapshot=initial.copy(catalog_digest="foreign")),
            valid.copy(snapshot=initial.copy(phase=UiSurfacePhase.ReadOnly)),
            valid.copy(snapshot=initial.copy(identity=initial.identity.copy(owner=initial.identity.owner.copy(participant_id="other")))))) {
            assertTrue(runCatching { bad.requireMatches(input,initial) }.isFailure)
        }
    }

    @Test fun coreGoldenProjectionEditsDeltaActionAndHistoryDecodeWithoutPlatformRuntime() {
        val source = checkNotNull(javaClass.classLoader?.getResourceAsStream("a2ui/form_interaction_v0_9_1.json"))
            .bufferedReader().use { it.readText() }
        val fixture = Json.parseToJsonElement(source).jsonObject
        assertEquals(1, fixture.getValue("fixture_version").jsonPrimitive.int)
        val initial = Json.decodeFromJsonElement(UiSnapshot.serializer(), fixture.getValue("initial_snapshot"))
        val edited = Json.decodeFromJsonElement(UiSnapshot.serializer(), fixture.getValue("edited_snapshot"))
        val changes = Json.decodeFromJsonElement(UiChangeSet.serializer(), fixture.getValue("changes"))
        val interaction = Json.decodeFromJsonElement(UiInteraction.serializer(), fixture.getValue("interaction"))
        val historical = Json.decodeFromJsonElement(UiSnapshot.serializer(), fixture.getValue("historical_snapshot"))
        assertEquals(edited, applyUiSurfaceUpdate(initial.identity, initial, UiSurfaceUpdate.Changes(changes)))
        assertEquals(initial.identity, interaction.identity)
        assertEquals(initial.revision, interaction.expected_surface_revision)
        assertEquals(initial.identity, historical.identity)
        assertEquals(edited.revision, historical.revision)
        assertEquals(UiSurfacePhase.ReadOnly, historical.phase)
        assertTrue(historical.nodes.values.all { it.bindings.isEmpty() && it.actions.isEmpty() })
        val event = fixture.getValue("resolved_event").jsonObject
        assertEquals("submit_form", event.getValue("name").jsonPrimitive.content)
        assertEquals("共同验收", event.getValue("context").jsonObject.getValue("name").jsonPrimitive.content)
        assertEquals(listOf("zh"), event.getValue("context").jsonObject.getValue("languages").jsonArray.map { it.jsonPrimitive.content })
        assertEquals(true, event.getValue("context").jsonObject.getValue("confirmed").jsonPrimitive.boolean)
        interaction.edits.forEach { edit ->
            require(edit is UiEdit.Set)
            assertEquals(edit.value, edited.nodes.getValue(edit.instance_id).props.getValue(edit.binding))
        }
        val changedIds = changes.upserted_nodes.map { it.instance_id }.toSet()
        val merged = checkNotNull(applyUiSurfaceUpdate(initial.identity, initial, UiSurfaceUpdate.Changes(changes)))
        initial.nodes.filterKeys { it !in changedIds }.forEach { (id, node) -> assertSame(node, merged.nodes[id]) }
        try { applyUiSurfaceUpdate(initial.identity, edited, UiSurfaceUpdate.Changes(changes)); fail("Stale delta") }
        catch (_: IllegalArgumentException) { }
        try {
            applyUiSurfaceUpdate(initial.identity.copy(owner = initial.identity.owner.copy(participant_id = "other")), initial, UiSurfaceUpdate.Changes(changes))
            fail("Foreign participant")
        } catch (_: IllegalArgumentException) { }
    }
}
