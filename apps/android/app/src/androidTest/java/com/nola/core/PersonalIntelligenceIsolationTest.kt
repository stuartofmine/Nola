// SPDX-License-Identifier: Apache-2.0
package com.nola.core

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.nola.data.local.AndroidApplicationRuntime
import kotlinx.serialization.json.jsonArray
import kotlin.io.path.createTempDirectory
import org.junit.Assert.*
import org.junit.Test

class PersonalIntelligenceIsolationTest {
    @Test fun clearingAnAdjacentRuntimePreservesTheOriginalMemory() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val root = createTempDirectory(context.cacheDir.toPath(), "private-intelligence-").toFile()
        try {
            AndroidApplicationRuntime(context, root.resolve("first.db")).use { first ->
                AndroidApplicationRuntime(context, root.resolve("second.db")).use { second ->
                    val a = PersonalIntelligenceCore(first)
                    val b = PersonalIntelligenceCore(second)
                    assertNotNull(a.ingest(IntelligenceIngestRequest(memory_candidates = listOf(
                        IntelligenceMemoryCandidate(semantic_key = "test.memory", statement = "Only in the first store", confidence = 0.95),
                    ))))
                    assertTrue(a.snapshot()!!.getValue("memories").jsonArray.toString().contains("Only in the first store"))
                    assertEquals(0, b.snapshot()!!.getValue("memories").jsonArray.size)
                    assertTrue(b.clear())
                    assertTrue(a.recallContext("Only in the first store").contains("Only in the first store"))
                }
            }
        } finally {
            root.deleteRecursively()
        }
    }
}
