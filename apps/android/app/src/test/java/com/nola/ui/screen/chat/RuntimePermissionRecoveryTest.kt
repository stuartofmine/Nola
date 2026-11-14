package com.nola.ui.screen.chat

import org.junit.Assert.*
import org.junit.Test

class RuntimePermissionRecoveryTest {
    @Test fun settingsRecoveryIsOnlyOfferedForDeniedPermissionsThatCannotBeRequestedAgain() {
        assertFalse(requiresRuntimePermissionSettings(emptyMap()) { false })
        assertFalse(requiresRuntimePermissionSettings(mapOf("contacts" to true)) { false })
        assertFalse(requiresRuntimePermissionSettings(mapOf("contacts" to false)) { true })
        assertTrue(requiresRuntimePermissionSettings(mapOf("contacts" to false)) { false })
        assertTrue(requiresRuntimePermissionSettings(mapOf("contacts" to true, "calendar" to false)) { false })
    }
}
