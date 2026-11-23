// SPDX-License-Identifier: Apache-2.0
package com.nola.llm.provider

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ModelRequestPrivacyNativeTest {
    @Test fun packagedAdapterRedactsResponsesToolOutputWithoutChangingInput() {
        val raw = """{"input":[{"type":"function_call_output","call_id":"call_9","output":"Email person@example.org. Bearer fixture-auth-token"}]}"""
        val input = Json.parseToJsonElement(raw).jsonObject
        val wire = ModelRequestPrivacy.redact(input, "fixture-auth-token").decodeToString()
        assertTrue(wire.contains("[EMAIL]") && wire.contains("[CREDENTIAL]") && wire.contains("call_9"))
        assertFalse(wire.contains("person@example.org") || wire.contains("fixture-auth-token"))
        assertEquals(raw, input.toString())
    }
}
