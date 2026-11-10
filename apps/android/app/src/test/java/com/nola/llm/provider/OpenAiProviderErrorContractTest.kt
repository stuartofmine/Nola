package com.nola.llm.provider

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class OpenAiProviderErrorContractTest {
    @Test
    fun structuredContextOverflowUsesCanonicalCoreCode() {
        assertEquals(
            "context_length_exceeded",
            openAiProviderErrorCode(
                """{"error":{"message":"输入超过模型上下文限制","type":"invalid_request_error","code":"context_length_exceeded"}}""",
            ),
        )
        assertEquals(
            "context_length_exceeded",
            openAiProviderErrorCode(
                """{"errors":[{"code":"gateway_failure"},{"message":"contexto demasiado largo","type":"input_too_long"}]}""",
            ),
        )
        assertEquals(
            "context_length_exceeded",
            openAiProviderErrorCode(
                """{"error":[{"message":"入力が長すぎます","type":"prompt_too_long"}]}""",
            ),
        )
        assertEquals(
            "context_length_exceeded",
            openAiProviderErrorCode(
                """{"error":{"code":"invalid_request","error_code":"context_length_error","message":"コンテキストを短くしてください"}}""",
            ),
        )
        assertEquals(
            "context_length_exceeded",
            openAiProviderErrorCode(
                """{"error":{"code":"gateway_failure"},"errors":[{"type":"invalid_request"},{"error_code":"request_too_large"}]}""",
            ),
        )
        val failure = llmRequestException(
            400,
            """{"error":{"message":"上下文过长","error_code":"request_too_large"}}""",
        )
        assertEquals("context_length_exceeded", failure.code)
        assertEquals(false, failure.retryable)
    }

    @Test
    fun proseAndRequestEchoCannotSelectOverflowRecovery() {
        assertNull(openAiProviderErrorCode("context_length_exceeded: 输入太长"))
        assertNull(
            openAiProviderErrorCode(
                """{"error":{"code":"invalid_request"},"request":{"code":"context_length_exceeded"}}""",
            ),
        )
        assertNull(
            openAiProviderErrorCode(
                """{"error":{"code":"invalid_request"},"request":{"error_code":"request_too_large"}}""",
            ),
        )
        assertNull(
            openAiProviderErrorCode(
                """{"error":{"message":"Please search the current context","code":"invalid_request"}}""",
            ),
        )
        assertNull(
            openAiProviderErrorCode(
                """{"error":{"code":{"value":"context_length_exceeded"},"type":["request_too_large"],"error_code":{"nested":true}}}""",
            ),
        )
        assertEquals(
            "context_length_exceeded",
            openAiProviderErrorCode(
                """{"errors":[{"code":{"malformed":true}},{"error_code":"request_too_large"}]}""",
            ),
        )
    }
}
