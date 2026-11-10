package com.nola

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertTrue
import org.junit.Test

class LocalizationKeyCoverageTest {
    @Test
    fun everySupportedLocaleDefinesExecutionUnitAndMessagingChannelKeys() {
        val resourceRoot = File("src/main/res")
        val requiredKeys = setOf(
            "chat_execution_unit",
            "chat_execution_unit_unavailable",
            "chat_execution_unit_not_installed",
            "chat_execution_unit_sign_in_required",
            "chat_execution_unit_incompatible",
            "models_host_messaging_title",
            "models_host_messaging_summary",
            "models_host_messaging_providers",
            "models_host_messaging_running",
            "models_host_messaging_stopped",
            "models_host_messaging_manage",
        )

        val localeFiles = resourceRoot.listFiles()
            .orEmpty()
            .filter { it.isDirectory && it.name.startsWith("values") }
            .map { File(it, "strings.xml") }
            .filter(File::isFile)

        assertTrue("No Android localization resources were found", localeFiles.isNotEmpty())
        localeFiles.forEach { file ->
            val document = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)
            val strings = document.getElementsByTagName("string")
            val keys = buildSet {
                for (index in 0 until strings.length) {
                    val name = strings.item(index).attributes?.getNamedItem("name")?.nodeValue
                    if (!name.isNullOrBlank()) add(name)
                }
            }
            val missing = requiredKeys - keys
            val locale = file.parentFile?.name ?: file.path
            assertTrue("$locale is missing: ${missing.sorted()}", missing.isEmpty())
        }
    }
}
