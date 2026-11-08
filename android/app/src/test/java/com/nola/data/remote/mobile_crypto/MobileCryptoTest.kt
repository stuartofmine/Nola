package com.nola.data.remote.mobile_crypto

import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class MobileCryptoTest {
    @Test
    fun encryptJsonPreservesNestedPayload() {
        val payload = mapOf<String, Any?>(
            "messages" to listOf(
                mapOf("role" to "user", "content" to "hello")
            ),
            "agent_mode" to true,
            "max_iterations" to 5,
            "permission_decisions" to listOf(
                mapOf(
                    "id" to "approval-1",
                    "tool_name" to "bash",
                    "match" to "ls",
                    "decision" to "allow",
                    "metadata" to mapOf("kind" to "nested")
                )
            )
        )

        val envelope = MobileCrypto.encryptJson(payload, token = "bridge-token")
        val decoded = MobileCrypto.decryptJsonObject(envelope, token = "bridge-token")

        val message = decoded["messages"]!!.jsonArray[0].jsonObject
        assertEquals("user", message["role"]!!.jsonPrimitive.content)
        assertEquals("hello", message["content"]!!.jsonPrimitive.content)
        assertEquals(true, decoded["agent_mode"]!!.jsonPrimitive.boolean)
        assertEquals(5, decoded["max_iterations"]!!.jsonPrimitive.int)

        val decision = decoded["permission_decisions"]!!.jsonArray[0].jsonObject
        assertEquals("approval-1", decision["id"]!!.jsonPrimitive.content)
        assertEquals("bash", decision["tool_name"]!!.jsonPrimitive.content)
        assertEquals("ls", decision["match"]!!.jsonPrimitive.content)
        assertEquals("allow", decision["decision"]!!.jsonPrimitive.content)
        assertEquals("nested", decision["metadata"]!!.jsonObject["kind"]!!.jsonPrimitive.content)
    }

    @Test
    fun decryptJsonReadsWebHostEnvelope() {
        val envelope = EncryptedEnvelope(
            nonce = "uqx9F8Kd2LZ1J3Mv",
            ciphertext = "ms_8nzO1WBdIGouXs9NM3j_3vihpnLuDPWmbLkmyhBgDA8lejsLcSqJ2cPCx8G-eW-6k11PU1ZP1awiyPhnbVfJNZ_0BXQSh-OO1OrFj90M6NIlrMQ",
            tag = "fzDBHmQZcODXE-dePAwjrQ"
        )

        val decoded = MobileCrypto.decryptJsonObject(envelope, token = "bridge-token")

        val message = decoded["messages"]!!.jsonArray[0].jsonObject
        assertEquals("user", message["role"]!!.jsonPrimitive.content)
        assertEquals("hello", message["content"]!!.jsonPrimitive.content)
        assertEquals(true, decoded["agent_mode"]!!.jsonPrimitive.boolean)
        assertEquals(5, decoded["max_iterations"]!!.jsonPrimitive.int)
    }

    @Test
    fun decryptJsonRejectsWrongToken() {
        val envelope = MobileCrypto.encryptJson(
            payload = mapOf("message" to "hello"),
            token = "bridge-token"
        )

        assertThrows(Exception::class.java) {
            MobileCrypto.decryptJsonObject(envelope, token = "wrong-token")
        }
    }
}
