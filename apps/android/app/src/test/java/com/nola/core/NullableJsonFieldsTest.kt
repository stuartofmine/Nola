package com.nola.core

import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import org.junit.Assert.assertEquals
import org.junit.Test

class NullableJsonFieldsTest {
    @Test
    fun nonNullAdmissionAndHostIdentityFieldsKeepTheirValues() {
        val fields = buildJsonObject {
            putNullableLong("expected_configuration_revision", 7L)
            putNullableString("expected_parent_message_id", "parent")
            putNullableString("attempt_id", "attempt-a")
            putNullableString("turn_id", "turn-a")
            putNullableString("permission_domain", "filesystem")
        }
        assertEquals(JsonPrimitive(7L), fields["expected_configuration_revision"])
        assertEquals(JsonPrimitive("parent"), fields["expected_parent_message_id"])
        assertEquals(JsonPrimitive("attempt-a"), fields["attempt_id"])
        assertEquals(JsonPrimitive("turn-a"), fields["turn_id"])
        assertEquals(JsonPrimitive("filesystem"), fields["permission_domain"])
    }

    @Test
    fun missingFieldsAreExplicitJsonNullAndReplacePreviousValues() {
        val keys = listOf("expected_parent_message_id", "attempt_id", "turn_id", "permission_domain")
        val fields = buildJsonObject {
            putNullableLong("expected_configuration_revision", 7L)
            putNullableLong("expected_configuration_revision", null)
            keys.forEach { key ->
                putNullableString(key, "previous")
                putNullableString(key, null)
            }
        }
        assertEquals(5, fields.size)
        (keys + "expected_configuration_revision").forEach { assertEquals(JsonNull, fields[it]) }
    }
}
