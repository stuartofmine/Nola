package com.nola.data.remote.api

import com.nola.core.AndroidAgentPermissionMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Test
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

class WebHostApiServiceImplTest {
    @Test fun tailcatStartupErrorsDoNotExposeNativeMessages() {
        assertEquals("native_host_inspection", tailcatStartupErrorCode(java.lang.reflect.InvocationTargetException(Exception("encrypted Nola Host inspection failed")), "native_connect"))
        assertEquals("native_host_dns", tailcatStartupErrorCode(java.lang.reflect.InvocationTargetException(Exception("encrypted Nola Host inspection failed (dns)")), "native_connect"))
        val secret = "fixture-secret-address-psk"
        val bounded = tailcatStartupErrorCode(java.lang.reflect.InvocationTargetException(Exception(secret)), "native_connect")
        assertEquals("native_connect:InvocationTargetException", bounded)
        assertFalse(bounded.contains(secret))
        assertEquals("sdk_binding", tailcatStartupErrorCode(NoSuchMethodException(secret), "native_connect"))
    }

    @Test
    fun webHostTransportPolicyLimitsHttpToPrivateNetworks() {
        assertFalse(isAllowedWebHostTransport("http://10.0.0.1.example.com:5001"))
        assertEquals(true, isAllowedWebHostTransport("https://host.example"))
        assertEquals(true, isAllowedWebHostTransport("http://127.0.0.1:5001"))
        assertEquals(true, isAllowedWebHostTransport("http://10.0.2.2:5001"))
        assertEquals(true, isAllowedWebHostTransport("http://172.31.4.2:5001"))
        assertEquals(true, isAllowedWebHostTransport("http://192.168.1.8:5001"))
        assertEquals(true, isAllowedWebHostTransport("http://nola.local:5001"))
        assertEquals(true, isAllowedWebHostTransport("http://[fd00::1]:5001"))
        assertEquals(false, isAllowedWebHostTransport("http://example.com:5001"))
        assertEquals(false, isAllowedWebHostTransport("http://8.8.8.8:5001"))
        assertEquals(false, isAllowedWebHostTransport("https://user:secret@example.com"))
    }

    @Test
    fun certificateFingerprintNormalizationAcceptsCommonDisplayFormats() {
        val raw = "SHA256/AA:BB:" + "CC".repeat(30)

        assertEquals("aabb" + "cc".repeat(30), normalizeCertificateSha256(raw))
        assertEquals("", normalizeCertificateSha256("  "))
        assertEquals(null, normalizeCertificateSha256("not-a-fingerprint"))
    }

    @Test
    fun mobileAuthProofBindsRequestPathWithoutSendingBearerToken() {
        val first = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/mobile/chat/agent/v2/stream",
            timestamp = 100,
            nonce = "mobile-nonce-123456"
        )
        val repeated = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/mobile/chat/agent/v2/stream",
            timestamp = 100,
            nonce = "mobile-nonce-123456"
        )
        val otherPath = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/mobile/other",
            timestamp = 100,
            nonce = "mobile-nonce-123456"
        )

        assertEquals(first, repeated)
        assertEquals("100", first["X-Nola-Mobile-Timestamp"])
        assertEquals("mobile-nonce-123456", first["X-Nola-Mobile-Nonce"])
        assertEquals(64, first["X-Nola-Mobile-Body-SHA256"]?.length)
        assertFalse(first.containsKey("Authorization"))
        assertNotEquals(first["X-Nola-Mobile-Auth"], otherPath["X-Nola-Mobile-Auth"])
        val otherBody = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/mobile/chat/agent/v2/stream",
            timestamp = 100,
            nonce = "mobile-nonce-123456",
            body = "different"
        )
        assertNotEquals(first["X-Nola-Mobile-Auth"], otherBody["X-Nola-Mobile-Auth"])
    }

    @Test
    fun webHostMobileUrlNormalizesCopiedMobileEndpoints() {
        assertEquals(
            "http://192.168.1.8:5001/api/mobile/chat/agent/v2/stream",
            webHostMobileUrl(
                "http://192.168.1.8:5001/api/mobile/login",
                "chat/agent/v2/stream"
            )
        )
        assertEquals(
            "http://192.168.1.8:5001/api/mobile/agent/settings",
            webHostMobileUrl(
                "192.168.1.8:5001/api/mobile/agent/settings?token=copy",
                "/agent/settings"
            )
        )
        assertEquals(
            "https://nola.local/api/mobile/chat/agent/v2/stream",
            webHostMobileUrl(
                "https://nola.local/api",
                "chat/agent/v2/stream"
            )
        )
        assertEquals(
            "https://nola.local/api/mobile/agent/settings",
            webHostMobileUrl("https://nola.local/v1", "agent/settings")
        )
    }

    @Test
    fun webHostControlPlaneUrlUsesOriginInsteadOfApiBasePath() {
        assertEquals(
            "https://nola.local:7443/models?tab=api",
            webHostControlPlaneUrl("https://nola.local:7443/api/mobile/login", "/models?tab=api")
        )
        assertEquals(
            "https://nola.local:7443/settings#channels",
            webHostControlPlaneUrl(
                "https://nola.local:7443/api/mobile/agent/settings",
                "/settings#channels"
            )
        )
        assertEquals(
            "https://nola.local/settings/channels?tab=active#telegram",
            webHostControlPlaneUrl(
                "https://nola.local/v1",
                "settings/channels?tab=active#telegram"
            )
        )
    }

    @Test
    fun browserHandoffPathKeepsSingleUseCodeOnHostOrigin() {
        assertEquals(
            "https://nola.local:7443/auth/mobile-handoff/one-time-code",
            webHostControlPlaneUrl(
                "https://nola.local:7443/api/mobile/agent/settings",
                "/auth/mobile-handoff/one-time-code"
            )
        )
    }

    @Test
    fun webHostMobileUrlPreservesConfiguredOriginIncludingExplicitEmulatorAlias() {
        assertEquals(
            "http://127.0.0.1:5001/api/mobile/agent/settings",
            webHostMobileUrl("http://127.0.0.1:5001/api/mobile/login", "agent/settings")
        )
        assertEquals(
            "http://localhost:5001/api/mobile/agent/settings",
            webHostMobileUrl("localhost:5001", "/agent/settings")
        )
        assertEquals(
            "http://[::1]:5001/api/mobile/agent/settings",
            webHostMobileUrl("http://[::1]:5001/api", "agent/settings")
        )
        assertEquals(
            "HTTP://127.0.0.1:5001/api/mobile/agent/settings",
            webHostMobileUrl("HTTP://127.0.0.1:5001/api", "agent/settings")
        )
        assertEquals(
            "http://10.0.2.2:5001/api/mobile/agent/settings",
            webHostMobileUrl("http://10.0.2.2:5001", "agent/settings")
        )
        assertEquals(
            "http://127.999.0.1:5001/api/mobile/agent/settings",
            webHostMobileUrl("http://127.999.0.1:5001/api", "agent/settings")
        )
    }

    @Test
    fun loopbackToolUrlsCanBeRewrittenAgainstReachableHost() {
        assertEquals(
            "http://192.168.1.8:3000/app",
            rewriteLoopbackHttpUrlForWebHost(
                "http://127.0.0.1:3000/app",
                "http://192.168.1.8:5001/api/mobile/login"
            )
        )
        assertEquals(
            "http://127.0.0.1:5001/static/call.png",
            rewriteLoopbackHttpUrlForWebHost(
                "http://localhost/static/call.png",
                "http://127.0.0.1:5001"
            )
        )
        assertEquals(
            "http://localhost/static/call.png",
            rewriteLoopbackHttpUrlForWebHost("http://localhost/static/call.png", "")
        )
        assertEquals(
            "https://example.com/static/call.png",
            rewriteLoopbackHttpUrlForWebHost(
                "https://example.com/static/call.png",
                "http://127.0.0.1:5001"
            )
        )
    }

    @Test
    fun requestBodyCarriesMobileAgentProtocolFields() {
        val bridge = JsonObject(mapOf("protocol_version" to JsonPrimitive(1)))

        val body = buildWebHostAgentRequestBody(
            messages = listOf(
                JsonObject(
                    mapOf(
                        "role" to JsonPrimitive("user"),
                        "content" to JsonPrimitive("hello"),
                    ),
                ),
            ),
            conversationId = "conversation-1",
            allowLocalDevTools = true,
            maxIterations = 30,
            timezone = "Asia/Shanghai",
            language = "zh-Hans",
            agentMode = true,
            permissionProfile = AndroidAgentPermissionMode.AllowAll,
            responseStyle = "casual",
            customSystemPrompt = "Prefer concise replies.",
            nativeAppHostId = "desktop-host",
            expectedHostId = "cli-host-paired-1234",
            clientId = "mobile-client-1234",
            deviceBridge = bridge
        )

        assertEquals("Prefer concise replies.", body["custom_system_prompt"])
        assertEquals("desktop-host", body["native_app_host_id"])
        assertEquals("conversation-1", body["conversationId"])
        assertEquals("zh-Hans", body["language"])
        assertEquals("cli-host-paired-1234", body["expected_host_id"])
        assertEquals("mobile-client-1234", body["client_id"])
        assertEquals(bridge, body["device_bridge"])
        assertEquals("allow_all", body["permission_profile"])
    }

    @Test
    fun requestBodyOmitsBlankPromptMetadata() {
        val body = buildWebHostAgentRequestBody(
            messages = listOf(
                JsonObject(
                    mapOf(
                        "role" to JsonPrimitive("user"),
                        "content" to JsonPrimitive("hello"),
                    ),
                ),
            ),
            conversationId = null,
            allowLocalDevTools = true,
            maxIterations = 30,
            timezone = "Asia/Shanghai",
            language = "   ",
            agentMode = true,
            responseStyle = "   ",
            customSystemPrompt = "   ",
            nativeAppHostId = "   ",
            expectedHostId = "   ",
            clientId = "   "
        )

        assertFalse(body.containsKey("custom_system_prompt"))
        assertFalse(body.containsKey("response_style"))
        assertFalse(body.containsKey("native_app_host_id"))
        assertFalse(body.containsKey("conversationId"))
        assertEquals("cautious", body["permission_profile"])
        assertFalse(body.containsKey("language"))
        assertFalse(body.containsKey("expected_host_id"))
        assertFalse(body.containsKey("client_id"))
    }
}
