package com.nola.core

import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObjectBuilder
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.put

// JsonObjectBuilder.put returns the previous value, not the value being inserted.
// Keep nullable wire fields explicit without chaining Elvis to put's return value.
internal fun JsonObjectBuilder.putNullableString(key: String, value: String?) {
    put(key, value?.let(::JsonPrimitive) ?: JsonNull)
}

internal fun JsonObjectBuilder.putNullableLong(key: String, value: Long?) {
    put(key, value?.let(::JsonPrimitive) ?: JsonNull)
}
