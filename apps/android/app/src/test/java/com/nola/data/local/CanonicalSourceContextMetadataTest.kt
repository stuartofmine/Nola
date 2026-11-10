package com.nola.data.local

import com.nola.data.repository.toCanonicalContextSource
import com.nola.domain.model.CustomDataSourceConfig
import kotlinx.serialization.json.*
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.nio.file.Files
import java.time.Instant

class CanonicalSourceContextMetadataTest {
    @Test
    fun canonicalProjectionPreservesAuthorizationAndObservationThroughContextAdapter() {
        val directory = Files.createTempDirectory("nola-context-metadata").toFile()
        try {
            for (authorization in listOf("authorized", "not_required", "denied")) {
                val canonical = CanonicalDataSource(
                    id = "source", name = "Fixture", kind = "http",
                    configuration = CanonicalDataSourceConfiguration("http", "http_url", "https://example.test/source"),
                    accessPolicy = "defaultEnabled",
                    authorization = CanonicalDataSourceAuthorization(authorization, observedAtMs = 500),
                    validation = CanonicalDataSourceValidation("not_validated", null, null, null, null, null),
                    readState = buildJsonObject {
                        put("status", "ready")
                        put("observed_at_ms", 1_000)
                        put("content_reference", JsonNull)
                    },
                    freshnessPolicy = CanonicalDataSourceFreshness(60, false),
                    defaultEnabled = true, lifecycle = "ready", capabilities = emptyList(),
                    privacy = "allow-cloud-context", revision = 3,
                    lastSuccessfulReadAtMs = 2_000, createdAtMs = 0, updatedAtMs = 2_000,
                    deletedAtMs = null,
                )
                val client = CanonicalDataSourceClient { request ->
                    assertEquals("list", request.getValue("command").jsonObject.getValue("type").jsonPrimitive.content)
                    Json.encodeToJsonElement(listOf(canonical))
                }
                val projection = AndroidCanonicalDataSources(client, directory).list().single()
                val context = projection.toCanonicalContextSource()
                assertEquals(authorization, context.authorization)
                assertEquals(Instant.ofEpochMilli(1_000).toString(), context.observedAt)
                assertEquals(2_000L, projection.lastSuccessfulReadAt)
            }
        } finally { directory.deleteRecursively() }
    }

    @Test
    fun unobservedUiDraftDoesNotInventAuthorizationOrFreshness() {
        val draft = CustomDataSourceConfig(name = "Draft", location = "https://example.test", lastSuccessfulReadAt = 2_000)
        assertNull(draft.toCanonicalContextSource().authorization)
        assertNull(draft.toCanonicalContextSource().observedAt)
    }
}
