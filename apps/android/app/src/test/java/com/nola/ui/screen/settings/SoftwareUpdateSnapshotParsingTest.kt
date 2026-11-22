package com.nola.ui.screen.settings

import java.io.IOException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

class SoftwareUpdateSnapshotParsingTest {
    private fun obj(raw: String) = Json.parseToJsonElement(raw).jsonObject

    @Test
    fun parsesUpToDateWithoutRelease() {
        val snapshot = parseSoftwareUpdateSnapshot(obj("""{"status":"up_to_date"}"""))

        assertEquals("up_to_date", snapshot.status)
        assertNull(snapshot.release)
    }

    @Test
    fun parsesValidDirectAndStoreReleases() {
        val direct = parseSoftwareUpdateSnapshot(
            obj(
                """
                {
                    "status": "available",
                    "release": {
                        "release_id": "rel-1",
                        "version": "1.1.0",
                        "build": 12,
                        "notes": "Bug fixes",
                        "artifact": {
                            "kind": "apk",
                            "url": "https://example.com/nola.apk"
                        }
                    }
                }
                """.trimIndent()
            )
        )
        assertEquals("available", direct.status)
        assertEquals("rel-1", direct.release?.releaseId)
        assertEquals("1.1.0", direct.release?.version)
        assertEquals(12L, direct.release?.build)
        assertEquals("Bug fixes", direct.release?.notes)
        assertEquals("apk", direct.release?.artifact?.kind)

        val store = parseSoftwareUpdateSnapshot(
            obj(
                """
                {
                    "status": "available",
                    "release": {
                        "version": "1.1.0",
                        "build": 12,
                        "artifact": {
                            "kind": "store",
                            "url": "https://play.google.com/store/apps/details?id=com.nola"
                        }
                    }
                }
                """.trimIndent()
            )
        )
        assertEquals("store", store.release?.artifact?.kind)
        assertEquals("https://play.google.com/store/apps/details?id=com.nola", store.release?.artifact?.url)
    }

    @Test
    fun rejectsMalformedReleasePayloadsWithIOExceptionInsteadOfJsonException() {
        assertThrows(IOException::class.java) {
            parseSoftwareUpdateSnapshot(obj("""{"status":"available","release":{"build":12}}"""))
        }
        assertThrows(IOException::class.java) {
            parseSoftwareUpdateSnapshot(obj("""{"status":"available","release":{"version":"1.1.0"}}"""))
        }
        assertThrows(IOException::class.java) {
            parseSoftwareUpdateSnapshot(obj("""{"status":"available","release":{"version":"1.1.0","build":12}}"""))
        }
        assertThrows(IOException::class.java) {
            parseSoftwareUpdateSnapshot(
                obj("""{"status":"available","release":{"version":"1.1.0","build":12,"artifact":{"kind":"store"}}}""")
            )
        }
        assertThrows(IOException::class.java) {
            parseSoftwareUpdateSnapshot(
                obj("""{"status":"available","release":{"version":"1.1.0","build":12,"artifact":{"kind":"apk"}}}""")
            )
        }
    }
}
