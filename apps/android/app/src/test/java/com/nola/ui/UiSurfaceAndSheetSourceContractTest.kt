package com.nola.ui

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class UiSurfaceAndSheetSourceContractTest {
    private fun resolveMainRoot(): File = listOf(
        File("src/main"),
        File("apps/android/app/src/main"),
    ).firstOrNull(File::isDirectory)
        ?: error("Could not locate src/main from ${File(".").absolutePath}")

    @Test
    fun everyModalBottomSheetSkipsPartiallyExpandedAndAppliesNavigationBarsPadding() {
        val uiRoot = File(resolveMainRoot(), "java/com/nola/ui")
        val files = uiRoot.walkTopDown().filter { it.isFile && it.extension == "kt" }.sortedBy { it.path }.toList()
        val sheetRegex = Regex("""\bModalBottomSheet\s*\(""")
        var checkedCallSites = 0

        val expertManagementPanelHasNavPadding = File(uiRoot, "screen/chat/ExpertManagementPanel.kt")
            .readText()
            .contains(".navigationBarsPadding()")
        assertTrue("ExpertManagementPanel must apply navigationBarsPadding()", expertManagementPanelHasNavPadding)

        for (file in files) {
            val text = file.readText()
            for (match in sheetRegex.findAll(text)) {
                checkedCallSites++
                val openParen = text.indexOf('(', match.range.first)
                val argsClose = findBalancedDelimiter(text, openParen, '(', ')')
                val args = text.substring(openParen + 1, argsClose)

                val sheetStateMatch = Regex("""sheetState\s*=\s*([^,\n]+(?:\([^)]*\))?)""").find(args)
                assertTrue(
                    "${file.name}: ModalBottomSheet call missing sheetState argument:\n$args",
                    sheetStateMatch != null,
                )
                val sheetStateExpr = sheetStateMatch!!.groupValues[1].trim()
                val resolvedExpr = if (sheetStateExpr.matches(Regex("""[A-Za-z_][A-Za-z0-9_]*"""))) {
                    val preceding = text.substring(0, match.range.first)
                    val decl = Regex("""\bval\s+$sheetStateExpr\s*=\s*([^\n]+(?:\n[^\n]+){0,12})""")
                        .findAll(preceding)
                        .lastOrNull()
                        ?.groupValues
                        ?.get(1)
                        .orEmpty()
                    decl
                } else {
                    sheetStateExpr
                }
                val configuresExpanded = resolvedExpr.contains("skipPartiallyExpanded = true") ||
                    resolvedExpr.contains("rememberGuardedEditorSheetState")
                assertTrue(
                    "${file.name}: ModalBottomSheet sheetState '$sheetStateExpr' must pass skipPartiallyExpanded = true or rememberGuardedEditorSheetState (resolved='$resolvedExpr')",
                    configuresExpanded,
                )

                val bodyOpen = skipWhitespaceAndLineBreaks(text, argsClose + 1)
                assertTrue(
                    "${file.name}: ModalBottomSheet must have a trailing content lambda",
                    bodyOpen < text.length && text[bodyOpen] == '{',
                )
                val bodyClose = findBalancedDelimiter(text, bodyOpen, '{', '}')
                val body = text.substring(bodyOpen + 1, bodyClose)
                val appliesNavPadding = body.contains(".navigationBarsPadding()") ||
                    body.contains("ExpertManagementPanel(")
                assertTrue(
                    "${file.name}: ModalBottomSheet content lambda must apply navigationBarsPadding()",
                    appliesNavPadding,
                )
            }
        }
        assertTrue("Expected at least 10 ModalBottomSheet call sites, found $checkedCallSites", checkedCallSites >= 10)
    }

    @Test
    fun settingsAndModelsScreensPassScrollKeyToNolaPage() {
        val uiRoot = File(resolveMainRoot(), "java/com/nola/ui")
        val targets = listOf(
            File(uiRoot, "screen/settings/SettingsScreen.kt"),
            File(uiRoot, "screen/models/ModelsScreen.kt"),
        )
        for (file in targets) {
            assertTrue("Missing ${file.path}", file.isFile)
            val text = file.readText()
            val matches = Regex("""\bNolaPage\s*\(""").findAll(text).toList()
            assertTrue("${file.name} must invoke NolaPage", matches.isNotEmpty())
            for (match in matches) {
                val openParen = text.indexOf('(', match.range.first)
                val closeParen = findBalancedDelimiter(text, openParen, '(', ')')
                val args = text.substring(openParen + 1, closeParen)
                assertTrue(
                    "${file.name}: NolaPage call must pass scrollKey= ($args)",
                    args.contains(Regex("""\bscrollKey\s*="""),
                    ),
                )
            }
        }
    }

    @Test
    fun themeProvidesLocalContentColorInNolaTheme() {
        val themeFile = File(resolveMainRoot(), "java/com/nola/ui/theme/Theme.kt")
        val text = themeFile.readText()
        val themeFnStart = text.indexOf("fun NolaTheme(")
        assertTrue("Theme.kt must define NolaTheme", themeFnStart >= 0)
        val bodyOpen = text.indexOf('{', text.indexOf(')', themeFnStart))
        val bodyClose = findBalancedDelimiter(text, bodyOpen, '{', '}')
        val body = text.substring(bodyOpen + 1, bodyClose)
        assertTrue(
            "NolaTheme must provide LocalContentColor",
            body.contains(Regex("""CompositionLocalProvider\s*\(\s*LocalContentColor\s+provides\b""")),
        )
    }

    @Test
    fun uiSourcesAndXmlValuesAvoidUnapprovedHardcodedColorsAndNonZeroTonalElevation() {
        val mainRoot = resolveMainRoot()
        val uiRoot = File(mainRoot, "java/com/nola/ui")
        val approvedColorFiles = setOf("Theme.kt", "DesignSystem.kt", "ExpertBubbleStyle.kt")
        val disallowedColorToken = Regex(
            """\bColor\.(Black|White|Red|Blue|Green|Gray|DarkGray|LightGray|Yellow|Cyan|Magenta)\b|\bColor\(\s*0x[0-9A-Fa-f]{6,8}\s*\)"""
        )
        val tonalElevationRegex = Regex("""\btonalElevation\s*=\s*([^\s,\)]+)""")

        val violations = mutableListOf<String>()

        for (file in uiRoot.walkTopDown().filter { it.isFile && it.extension == "kt" }.sortedBy { it.path }) {
            val text = file.readText()
            val lines = text.lines()

            for (match in tonalElevationRegex.findAll(text)) {
                val value = match.groupValues[1]
                if (value != "0.dp") {
                    val lineNo = text.substring(0, match.range.first).count { it == '\n' } + 1
                    violations += "${file.name}:$lineNo uses non-zero tonalElevation=$value"
                }
            }

            if (file.name in approvedColorFiles) continue

            val switchBlockRange = if (file.name == "CommonComponents.kt") {
                val idx = text.indexOf("SwitchDefaults.colors(")
                if (idx >= 0) {
                    val open = text.indexOf('(', idx)
                    open..findBalancedDelimiter(text, open, '(', ')')
                } else null
            } else null

            for (match in disallowedColorToken.findAll(text)) {
                val offset = match.range.first
                val lineNo = text.substring(0, offset).count { it == '\n' } + 1
                val lineText = lines.getOrElse(lineNo - 1) { "" }
                val token = match.value

                val approvedShadow = file.name == "NolaSurface.kt" &&
                    (lineText.contains("ambientColor =") || lineText.contains("spotColor =")) &&
                    token == "Color.Black"
                val approvedCodeSpan = file.name == "MarkdownRenderer.kt" &&
                    token.replace(" ", "") == "Color(0x20000000)"
                val approvedSwitch = switchBlockRange != null && offset in switchBlockRange

                if (!approvedShadow && !approvedCodeSpan && !approvedSwitch) {
                    violations += "${file.name}:$lineNo has unapproved hardcoded color '$token': ${lineText.trim()}"
                }
            }
        }

        val resRoot = File(mainRoot, "res")
        for (file in listOf(
            File(resRoot, "values/colors.xml"),
            File(resRoot, "values/themes.xml"),
            File(resRoot, "values-night/themes.xml"),
        )) {
            if (!file.isFile) continue
            val xml = file.readText()
            assertFalse("${file.path} must not use pure OLED black #000000", xml.contains("#000000"))
            assertFalse("${file.path} must not use cold dark gray #121212", xml.contains("#121212"))
            assertFalse("${file.path} must not use cold gray #1E1E1E", xml.contains("#1E1E1E", ignoreCase = true))
        }
        assertEquals(
            "Found UI color/tonalElevation contract violations:\n${violations.joinToString("\n")}",
            emptyList<String>(),
            violations,
        )
    }

    private fun skipWhitespaceAndLineBreaks(source: String, start: Int): Int {
        var idx = start
        while (idx < source.length && source[idx].isWhitespace()) idx++
        return idx
    }

    private fun findBalancedDelimiter(source: String, openIndex: Int, open: Char, close: Char): Int {
        var depth = 0
        var i = openIndex
        var inString = false
        var escape = false
        while (i < source.length) {
            val ch = source[i]
            if (inString) {
                if (escape) {
                    escape = false
                } else if (ch == '\\') {
                    escape = true
                } else if (ch == '"') {
                    inString = false
                }
            } else {
                when (ch) {
                    '"' -> inString = true
                    open -> depth++
                    close -> {
                        depth--
                        if (depth == 0) return i
                    }
                }
            }
            i++
        }
        error("Unbalanced '$open'..'$close' starting at $openIndex")
    }
}
