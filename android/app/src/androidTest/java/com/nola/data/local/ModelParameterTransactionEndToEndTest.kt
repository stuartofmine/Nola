package com.nola.data.local

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.data.local.datastore.ModelParametersStore
import java.io.File
import java.util.UUID
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ModelParameterTransactionEndToEndTest {
    @Test
    fun versionedReceiptCrossesJniAndReplaysAfterReopen() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val root = File(context.cacheDir, "model-cas-${UUID.randomUUID()}")
        val database = File(root, "state.db")
        val input = CanonicalModelProfile(
            id = "cas", displayName = "CAS", provider = "test", modelName = "cas",
            runtime = "android", sourceJson = "{}",
            parametersJson = Json.encodeToString(ModelParametersStore.defaultParameters),
            selected = false, enabled = true, revision = 0, createdAtMs = 1,
            updatedAtMs = 1, deletedAtMs = null,
        )
        try {
            val revision = AndroidApplicationRuntime(context, database).use { runtime ->
                val revision = runtime.configureModelParameters(input, 0, "create")
                assertTrue(revision > 0)
                assertEquals(revision, runtime.configureModelParameters(input, 0, "create"))
                val failure = runCatching { runtime.configureModelParameters(input, 0, "stale") }.exceptionOrNull()
                assertTrue(failure is AndroidApplicationRuntime.RevisionConflict)
                assertEquals(revision, (failure as AndroidApplicationRuntime.RevisionConflict).actual)
                assertEquals("model_profile:cas", failure.scope)
                revision
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                assertEquals(revision, runtime.configureModelParameters(input, 0, "create"))
                assertEquals(revision, runtime.modelProfile("cas")!!.revision)
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun parameterEditsPreserveCanonicalIdentityAndCannotRegisterOrRestoreModels() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val root = File(context.cacheDir, "model-edit-${UUID.randomUUID()}")
        val database = File(root, "product.db")
        val defaults = ModelParametersStore.defaultParameters
        val edited = defaults.copy(temperature = 0.42f)
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val store = ModelParametersStore(runtime)
                assertTrue(runCatching { store.saveParameters("missing", edited) }.isFailure)
                assertNull(runtime.modelProfile("missing"))
                runtime.upsertModelProfile(CanonicalModelProfile(
                    id = "existing", displayName = "Canonical name", provider = "test",
                    modelName = "canonical-model", runtime = "android",
                    sourceJson = "{\"endpoint\":\"canonical\"}",
                    parametersJson = Json.encodeToString(defaults), selected = true, enabled = true,
                    revision = 0, createdAtMs = 1, updatedAtMs = 1, deletedAtMs = null,
                ))
                val before = runtime.modelProfile("existing")!!
                store.saveParameters("existing", edited)
                val after = runtime.modelProfile("existing")!!
                assertEquals(before.sourceJson, after.sourceJson)
                assertEquals(before.displayName, after.displayName)
                assertEquals(before.modelName, after.modelName)
                assertEquals(before.provider, after.provider)
                assertEquals(before.runtime, after.runtime)
                assertEquals(before.createdAtMs, after.createdAtMs)
                assertTrue(after.selected)
                assertTrue(after.revision > before.revision)
                val first = store.parameterSnapshot("existing")
                val second = store.parameterSnapshot("existing")
                store.saveParameters(first, edited.copy(temperature = 0.2f))
                val conflict = runCatching { store.saveParameters(second, edited) }.exceptionOrNull()
                assertTrue(conflict is AndroidApplicationRuntime.RevisionConflict)
                assertEquals(0.2f, store.loadParameters("existing").temperature)
                store.saveParameters(store.parameterSnapshot("existing"), edited)
            }
            AndroidApplicationRuntime(context, database).use { reopened ->
                val store = ModelParametersStore(reopened)
                assertEquals(edited, store.loadParameters("existing"))
                assertTrue(reopened.modelProfile("existing")!!.selected)
                assertTrue(reopened.deleteModelProfile("existing", System.currentTimeMillis()))
                assertTrue(runCatching { store.saveParameters("existing", defaults) }.isFailure)
                assertNull(reopened.modelProfile("existing"))
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun resetThroughProductionStorePersistsAndFailedBatchRollsBack() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val root = File(context.cacheDir, "model-transaction-${UUID.randomUUID()}")
        val database = File(root, "product.db")
        val defaults = Json.encodeToString(ModelParametersStore.defaultParameters)
        val customized = Json.encodeToString(ModelParametersStore.defaultParameters.copy(temperature = 0.31f))
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                for ((id, selected) in listOf("first" to true, "second" to false)) {
                    runtime.upsertModelProfile(CanonicalModelProfile(
                        id = id, displayName = id, provider = "test", modelName = id,
                        runtime = "android", sourceJson = "{\"endpoint\":\"fixture\"}",
                        parametersJson = customized, selected = selected, enabled = true,
                        revision = 0, createdAtMs = 1, updatedAtMs = 1, deletedAtMs = null,
                    ))
                }
                var rejected = false
                try {
                    runtime.resetModelParameters(listOf("first", "missing"), defaults, 2)
                } catch (_: IllegalStateException) {
                    rejected = true
                }
                assertTrue("A missing model must reject the complete batch", rejected)
                assertEquals(customized, runtime.modelProfile("first")?.parametersJson)
                ModelParametersStore(runtime).clearAll()
            }
            AndroidApplicationRuntime(context, database).use { reopened ->
                val profiles = reopened.listModelProfiles()
                assertEquals(2, profiles.size)
                assertEquals(listOf("first"), profiles.filter { it.selected }.map { it.id })
                profiles.forEach {
                    assertEquals(defaults, it.parametersJson)
                    assertEquals("{\"endpoint\":\"fixture\"}", it.sourceJson)
                    assertTrue(it.enabled)
                }
            }
        } finally {
            root.deleteRecursively()
        }
    }
}
