package com.nola

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LocalizationKeyCoverageTest {
    private val supportedLocales = listOf(
        "values-de",
        "values-es",
        "values-fr",
        "values-ja",
        "values-ru",
        "values-zh",
    )

    private val nonTranslatableConfigFiles = setOf("colors.xml", "themes.xml")
    private val printfSpecRegex = Regex("""%(?:(\d+)\$)?([-#+ 0,(]*\d*(?:\.\d+)?[bBhHsScCdoxXeEfgGaAtT%])""")

    private data class StringEntry(
        val fileName: String,
        val translatable: Boolean,
        val formatted: Boolean,
        val value: String,
    )

    private data class LocaleResources(
        val fileNames: Set<String>,
        val strings: Map<String, StringEntry>,
        val plurals: Map<String, Map<String, String>>,
    )

    @Test
    fun allSixLocalesHaveCompleteStringAndPluralParityAndMatchingFormatPlaceholders() {
        val resRoot = listOf(
            File("src/main/res"),
            File("apps/android/app/src/main/res"),
        ).firstOrNull(File::isDirectory)
            ?: error("Could not locate Android res directory from ${File(".").absolutePath}")

        val base = parseLocaleDirectory(File(resRoot, "values"))
        assertTrue("Expected base XML string resources", base.strings.isNotEmpty())
        assertTrue("Expected base XML plurals resources", base.plurals.isNotEmpty())

        val requiredStringKeys = base.strings.filterValues { it.translatable }.keys
        val requiredPluralKeys = base.plurals.keys

        for (locale in supportedLocales) {
            val localeDir = File(resRoot, locale)
            assertTrue("Missing localized resource directory: $locale", localeDir.isDirectory)

            val localized = parseLocaleDirectory(localeDir)
            val missingFiles = base.fileNames - localized.fileNames
            val extraFiles = localized.fileNames - base.fileNames
            assertTrue("$locale missing XML files: ${missingFiles.sorted()}", missingFiles.isEmpty())
            assertTrue("$locale has extra XML files: ${extraFiles.sorted()}", extraFiles.isEmpty())

            val missingStrings = requiredStringKeys - localized.strings.keys
            val extraStrings = localized.strings.keys - base.strings.keys
            assertTrue("$locale missing <string> keys: ${missingStrings.sorted()}", missingStrings.isEmpty())
            assertTrue("$locale has stale <string> keys: ${extraStrings.sorted()}", extraStrings.isEmpty())

            val missingPlurals = requiredPluralKeys - localized.plurals.keys
            val extraPlurals = localized.plurals.keys - requiredPluralKeys
            assertTrue("$locale missing <plurals> keys: ${missingPlurals.sorted()}", missingPlurals.isEmpty())
            assertTrue("$locale has stale <plurals> keys: ${extraPlurals.sorted()}", extraPlurals.isEmpty())

            for (key in requiredStringKeys) {
                val baseEntry = base.strings.getValue(key)
                val locEntry = localized.strings.getValue(key)
                assertTrue("$locale/$key has empty text", locEntry.value.isNotBlank())
                if (baseEntry.formatted && locEntry.formatted) {
                    val basePlaceholders = extractPlaceholders(baseEntry.value)
                    val locPlaceholders = extractPlaceholders(locEntry.value)
                    assertEquals(
                        "$locale/$key (${locEntry.fileName}) format placeholders differ from base (${baseEntry.fileName}): base='$basePlaceholders' vs loc='$locPlaceholders'",
                        basePlaceholders,
                        locPlaceholders,
                    )
                }
            }

            for (key in requiredPluralKeys) {
                val baseItems = base.plurals.getValue(key)
                val locItems = localized.plurals.getValue(key)
                assertTrue("$locale/$key has no plural <item> entries", locItems.isNotEmpty())
                val basePlaceholderSignatures = baseItems.values.map(::extractPlaceholders).toSet()
                for ((quantity, text) in locItems) {
                    assertTrue("$locale/$key[$quantity] has empty text", text.isNotBlank())
                    val locSignature = extractPlaceholders(text)
                    assertTrue(
                        "$locale/$key[$quantity] format placeholders $locSignature do not match any base form $basePlaceholderSignatures",
                        locSignature in basePlaceholderSignatures,
                    )
                }
            }
        }
    }

    private fun parseLocaleDirectory(directory: File): LocaleResources {
        val factory = DocumentBuilderFactory.newInstance()
        val xmlFiles = directory.listFiles()
            .orEmpty()
            .filter { it.isFile && it.extension == "xml" && it.name !in nonTranslatableConfigFiles }
            .sortedBy { it.name }

        val fileNames = xmlFiles.mapTo(mutableSetOf()) { it.name }
        val strings = linkedMapOf<String, StringEntry>()
        val plurals = linkedMapOf<String, Map<String, String>>()

        for (file in xmlFiles) {
            val doc = factory.newDocumentBuilder().parse(file)
            val stringNodes = doc.getElementsByTagName("string")
            for (i in 0 until stringNodes.length) {
                val element = stringNodes.item(i)
                val attrs = element.attributes ?: continue
                val name = attrs.getNamedItem("name")?.nodeValue?.takeIf(String::isNotBlank) ?: continue
                val translatable = attrs.getNamedItem("translatable")?.nodeValue != "false"
                val formatted = attrs.getNamedItem("formatted")?.nodeValue != "false"
                val value = element.textContent.orEmpty()
                assertTrue(
                    "Duplicate <string name=\"$name\"> in ${directory.name}/${file.name}",
                    name !in strings,
                )
                strings[name] = StringEntry(file.name, translatable, formatted, value)
            }

            val pluralNodes = doc.getElementsByTagName("plurals")
            for (i in 0 until pluralNodes.length) {
                val element = pluralNodes.item(i)
                val name = element.attributes?.getNamedItem("name")?.nodeValue?.takeIf(String::isNotBlank)
                    ?: continue
                val items = linkedMapOf<String, String>()
                val children = element.childNodes
                for (j in 0 until children.length) {
                    val child = children.item(j)
                    if (child.nodeName == "item") {
                        val quantity = child.attributes?.getNamedItem("quantity")?.nodeValue ?: continue
                        items[quantity] = child.textContent.orEmpty()
                    }
                }
                assertTrue(
                    "Duplicate <plurals name=\"$name\"> in ${directory.name}/${file.name}",
                    name !in plurals,
                )
                plurals[name] = items
            }
        }

        return LocaleResources(fileNames = fileNames, strings = strings, plurals = plurals)
    }

    private fun extractPlaceholders(text: String): List<Pair<Int, Char>> {
        var nextPositional = 1
        return buildList {
            for (match in printfSpecRegex.findAll(text)) {
                val tail = match.groupValues[2]
                if (tail == "%") continue
                val explicitPos = match.groupValues[1].toIntOrNull()
                val position = explicitPos ?: nextPositional++
                val conversion = tail.last()
                add(position to conversion)
            }
        }.sortedWith(compareBy({ it.first }, { it.second }))
    }
}
