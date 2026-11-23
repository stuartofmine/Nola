package com.nola.ui.components

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PhotoCamera
import androidx.compose.material.icons.filled.Terminal
import com.nola.R
import org.junit.Assert.assertEquals
import org.junit.Test

class HostToolDisplayNameTest {
    @Test
    fun storedTitlesResolveFromPackagedCatalogAndCustomNamesRemainAvailable() {
        assertEquals("코드 실행", BuiltinToolNames.label("code_mode_exec", "执行代码", java.util.Locale.KOREAN))
        assertEquals("顯示互動介面", BuiltinToolNames.label("render_ui", null, java.util.Locale.TAIWAN))
        assertEquals(null, BuiltinToolNames.label("render_ui", "My custom panel", java.util.Locale.GERMAN))
    }

    @Test
    fun fileToolAliasesShareOneLocalizedDisplayName() {
        listOf("read_file", "read-file", "Read file", "read   file").forEach { alias ->
            assertEquals(
                R.string.chat_host_tool_name_read_file,
                hostToolDisplayNameStringRes(alias),
            )
        }
    }

    @Test
    fun skillToolNamesShareTheSkillDisplayName() {
        listOf("Skill", "skill", "skill_context").forEach { alias ->
            assertEquals(
                R.string.chat_host_tool_name_skill,
                hostToolDisplayNameStringRes(alias),
            )
        }
    }

    @Test
    fun ptyToolsHaveCleanLabelsAndTerminalOrCameraIcons() {
        assertEquals(R.string.chat_host_tool_name_pty_exec, hostToolDisplayNameStringRes("pty_exec"))
        assertEquals(R.string.chat_host_tool_name_pty_write, hostToolDisplayNameStringRes("pty_write"))
        assertEquals(R.string.chat_host_tool_name_pty_screenshot, hostToolDisplayNameStringRes("pty_screenshot"))
        assertEquals(R.string.chat_host_tool_name_pty_close, hostToolDisplayNameStringRes("pty_close"))

        assertEquals(Icons.Filled.Terminal, hostToolIcon("pty_exec"))
        assertEquals(Icons.Filled.Terminal, hostToolIcon("pty_write"))
        assertEquals(Icons.Filled.Terminal, hostToolIcon("pty_close"))
        assertEquals(Icons.Filled.PhotoCamera, hostToolIcon("pty_screenshot"))
    }
}
