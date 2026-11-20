// SPDX-License-Identifier: Apache-2.0
package com.nola.core

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.security.SecureTokenStore
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File
import java.util.UUID

class AndroidGithubSkillPackageTest {
    @Test fun publicPackageSurvivesReopenAndEnforcesActivation() = runBlocking {
        assumeTrue(InstrumentationRegistry.getArguments().getString("liveGithub") == "true")
        val context = ApplicationProvider.getApplicationContext<Context>()
        val root = File(context.cacheDir, "github-skill-${UUID.randomUUID()}").apply { mkdirs() }
        val skill = "nature-paper-card"
        val url = "https://github.com/Yuan1z0825/nature-skills/tree/main/skills/$skill"
        val activatedPackageRefs = mutableSetOf<String>()
        suspend fun call(store: MemoryProfileDataStore, tool: String, vararg args: Pair<String,String>): AndroidAgentHostCallResult {
            val executor = AndroidAgentHostExecutor(context, store)
            val plan = AgentCoreToolCatalog.plan(AndroidAgentExecutionMode.DeviceLocal, AndroidAgentPermissionMode.AllowAll,
                AndroidAgentPermissionBroker(context), executor, AgentCoreRuntimeIdentity("android", "test", "session"), true, JsonObject(emptyMap()))
            val enabled = requireNotNull(plan.allowedTools.singleOrNull { it.toolId == tool }) { "Missing local $tool in ${plan.allowedTools}" }
            return executor.execute(AndroidAgentHostCallRequest(UUID.randomUUID().toString(), tool, enabled.capability,
                buildJsonObject { args.forEach { (k,v) -> put(k,v) } }, null), plan, activatedPackageRefs)
        }
        try {
            AndroidApplicationRuntime(context, File(root, "product.db")).use { app ->
                val store = MemoryProfileDataStore(context, SecureTokenStore(context), app)
                val choices = call(store, "device_github_skill_find", "query" to "nature-skills")
                assertFalse(choices.output.toString(), choices.isError)
                assertTrue(choices.output.toString().contains("Yuan1z0825/nature-skills", ignoreCase = true))
                val installed = call(store, "device_github_skill_install", "url" to url)
                assertFalse(installed.output.toString(), installed.isError)
                assertEquals("installed", installed.output.jsonObject.getValue("status").jsonPrimitive.content)
                assertTrue(store.skills().single().enabled)
            }
            AndroidApplicationRuntime(context, File(root, "product.db")).use { app ->
                val store = MemoryProfileDataStore(context, SecureTokenStore(context), app)
                val item = store.skills().single()
                assertTrue(item.canonicalSkillRecord!!.getValue("instructions_ref").jsonPrimitive.content.startsWith("nola://github-skills/"))
                store.saveSkill(store.skillEditor(item), item.copy(description = "Edited source"))
                assertEquals("Edited source", store.skills().single().description)
                store.setSkillEnabled(store.skillEditor(skill), false)
                assertTrue(call(store, "device_skill_read_resource", "skill" to skill, "path" to "SKILL.md").isError)
                store.setSkillEnabled(store.skillEditor(skill), true)
                assertTrue(call(store, "device_skill_read_resource", "skill" to skill, "path" to "SKILL.md").isError)
                val invoked = call(store, "skill_invoke", "skill" to skill)
                assertFalse(invoked.output.toString(), invoked.isError)
                assertEquals("device_skill_read_resource", invoked.output.jsonObject.getValue("_nola_execution_context").jsonObject
                    .getValue("resource_reader").jsonPrimitive.content)
                val base = File(invoked.output.jsonObject.getValue("base_directory").jsonPrimitive.content)
                assertTrue(File(base, "scripts/prepare_paper.py").isFile)
                val read = call(store, "device_skill_read_resource", "skill" to skill, "path" to "../nature-shared/core/terminology-ledger.md")
                assertFalse(read.output.toString(), read.isError)
                assertTrue(read.output.jsonObject.getValue("content").jsonPrimitive.content.contains("Terminology Ledger"))
                assertTrue(call(store, "device_skill_read_resource", "skill" to skill, "path" to "../../../../product.db").isError)
            }
        } finally { root.deleteRecursively() }
    }
}
