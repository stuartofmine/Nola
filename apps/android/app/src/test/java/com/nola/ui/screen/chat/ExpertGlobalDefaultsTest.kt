package com.nola.ui.screen.chat

import com.nola.data.local.CanonicalModelProfile
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class ExpertGlobalDefaultsTest {
    @Test fun inheritedEffortFactsUseCanonicalGlobalPreferenceAndNeverParentSelection() {
        val profile = CanonicalModelProfile("global", "Global", "fixture", "fixture", "remote", "{}", "{}", true, true, 1, 0, 0)
        val preference = """{"version":1,"selection":{"kind":"profile","profile_id":"global"}}"""
        val input = expertDefaultModelResolutionCommand(listOf(profile, profile.copy(id = "deleted", deletedAtMs = 1)), preference)
        assertEquals(JsonPrimitive("inherit"), input.getValue("intent").jsonObject["kind"])
        assertEquals(JsonPrimitive(preference), input["default_value_json"])
        assertEquals(listOf("global"), input.getValue("candidates").jsonArray.map { it.jsonObject.getValue("profile_id").jsonPrimitive.content })
        assertFalse(input.toString().contains("parent"))
        assertEquals("global", resolvedExpertDefaultProfileId(buildJsonObject { put("kind", "resolved"); put("profile_id", "global") }))
        assertNull(resolvedExpertDefaultProfileId(buildJsonObject { put("kind", "unavailable"); put("profile_id", "global") }))
    }
}
