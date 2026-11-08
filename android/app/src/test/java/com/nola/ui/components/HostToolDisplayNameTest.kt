package com.nola.ui.components

import com.nola.R
import org.junit.Assert.assertEquals
import org.junit.Test

class HostToolDisplayNameTest {
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
}
