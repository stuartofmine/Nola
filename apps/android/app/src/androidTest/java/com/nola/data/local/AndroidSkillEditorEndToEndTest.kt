package com.nola.data.local

import androidx.test.core.app.ApplicationProvider
import com.nola.domain.model.UploadedContent
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.*
import org.junit.Test
import java.io.File

class AndroidSkillEditorEndToEndTest {
    @Test
    fun sameDisplayNameDoesNotMergeIndependentSkills() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "skill-identities-${System.nanoTime()}")
        try {
            AndroidApplicationRuntime(context, File(root, "product.db")).use { runtime ->
                val first = runtime.createUserSkill(UploadedContent(name = "Review", content = "First workflow"))
                val second = runtime.createUserSkill(UploadedContent(name = "Review", content = "Second workflow"))
                assertNotEquals(first.record["id"], second.record["id"])
                runtime.configureUserSkill(first, "Renamed", "First workflow")
                assertEquals(2, runtime.listSkills().size)
                assertTrue(runtime.listSkills().any { it.name == "Review" && it.content == "Second workflow" })
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun canonicalEditorRejectsStaleChangesAndPreservesMetadataAfterReopen() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "skill-editor-${System.nanoTime()}")
        val database = File(root, "product.db")
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                runtime.createUserSkill(UploadedContent(
                    name = "Review", content = "Review the supplied text.",
                    resourceId = "review", description = "Keep this description",
                    requiredTools = listOf("read_file")
                ))
                val original = requireNotNull(runtime.skillEditor("review"))
                val edited = runtime.configureUserSkill(original, "Renamed", "Review carefully.")
                assertTrue(edited.revision > original.revision)
                assertEquals(original.record["required_tools"], edited.record["required_tools"])
                assertEquals(original.record["summary"], edited.record["summary"])
                assertEquals(original.record["created_at_ms"], edited.record["created_at_ms"])
                assertNotEquals(original.record["version"], edited.record["version"])
                for (operation in listOf<() -> Unit>(
                    { runtime.configureUserSkill(original, "Stale", "Stale text") },
                    { runtime.setSkillEnabled(original, false) },
                    { runtime.deleteUserSkill(original) }
                )) {
                    val error = runCatching(operation).exceptionOrNull()
                    assertTrue(error?.message.orEmpty().contains("skill_revision_conflict"))
                }
                assertEquals(edited, runtime.skillEditor("review"))
                val disabled = runtime.setSkillEnabled(edited, false)
                assertEquals("false", disabled.record.getValue("enabled").jsonPrimitive.content)
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                val current = requireNotNull(runtime.skillEditor("review"))
                assertEquals("Renamed", current.record.getValue("display_name").jsonPrimitive.content)
                assertEquals("false", current.record.getValue("enabled").jsonPrimitive.content)
                runtime.deleteUserSkill(current)
                assertTrue(runtime.listSkills().isEmpty())
                val error = runCatching {
                    runtime.configureUserSkill(current, "Resurrect", "Do not restore deleted content")
                }.exceptionOrNull()
                assertTrue(error?.message.orEmpty().contains("deleted"))
            }
        } finally {
            root.deleteRecursively()
        }
    }
}
