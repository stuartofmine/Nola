package com.nola.data.repository

import org.junit.Assert.*
import org.junit.Test

class ExpertRoleTemplatesTest {
    private val role = ExpertRoleTemplate("9f7d0000-4e6f-4c61-8000-000000000001", "code-writer",
        mapOf("zh" to "代码编写", "en" to "Code writer"), mapOf("en" to "Implement changes"), "Inspect before editing",
        ExpertRoleToolPack("coding", listOf("read_file", "write_file")), listOf(ExpertRoleSkill("implement-code", mapOf("en" to "Implementation"), "Verify actual changes")))
    private val catalog = ExpertRoleCatalog(1, ExpertRoleFeatures(true, true, true), listOf(role))
    @Test fun localizedPreviewUsesLanguageFallbackWithoutInventingRoleBodies() {
        assertEquals("代码编写", expertRoleText(role.name, "zh-Hans"))
        assertEquals("Code writer", expertRoleText(role.name, "ja"))
        assertEquals(catalog, catalog.validated())
        assertEquals("Verify actual changes", catalog.templates.single().skills.single().instructions)
    }
    @Test fun unsupportedVersionsFeaturesAndAmbiguousIdsAreRejected() {
        assertThrows(IllegalArgumentException::class.java) { catalog.copy(version = 2).validated() }
        assertThrows(IllegalArgumentException::class.java) { catalog.copy(features = catalog.features.copy(rejoin = false)).validated() }
        assertThrows(IllegalArgumentException::class.java) { catalog.copy(templates = listOf(role, role)).validated() }
        assertThrows(IllegalArgumentException::class.java) { catalog.copy(templates = listOf(role.copy(prompt = ""))).validated() }
    }
}
