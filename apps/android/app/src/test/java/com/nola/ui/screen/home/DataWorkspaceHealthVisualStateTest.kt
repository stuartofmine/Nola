package com.nola.ui.screen.home

import com.nola.data.health.AndroidHealthAvailability
import com.nola.data.health.AndroidHealthPermissionState
import com.nola.data.health.AndroidHealthSummary
import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Test

class DataWorkspaceHealthVisualStateTest {
    @Test
    fun missingSnapshotIsUpdating() {
        assertEquals(SourceVisualState.Updating, healthVisualState(DataSourcesUiState()))
    }

    @Test
    fun availableSnapshotRequiresPermissionBeforeShowingReady() {
        val summary = summary(permission = AndroidHealthPermissionState.Missing)
        assertEquals(SourceVisualState.NeedsPermission, healthVisualState(DataSourcesUiState(health = summary)))
    }

    @Test
    fun grantedSnapshotDistinguishesReadyPartialAndError() {
        assertEquals(SourceVisualState.Ready, healthVisualState(DataSourcesUiState(health = summary())))
        assertEquals(
            SourceVisualState.Partial,
            healthVisualState(DataSourcesUiState(health = summary(missingPermissionCount = 1)))
        )
        assertEquals(
            SourceVisualState.Error,
            healthVisualState(DataSourcesUiState(health = summary(error = "read failed")))
        )
    }

    @Test
    fun unavailableProviderNeverLooksReady() {
        val summary = summary(availability = AndroidHealthAvailability.Unavailable)
        assertEquals(SourceVisualState.Unavailable, healthVisualState(DataSourcesUiState(health = summary)))
    }

    @Test
    fun firstRefreshKeepsUpdatingUntilSnapshotHasTimestamp() {
        val summary = summary(updatedAt = null)
        assertEquals(
            SourceVisualState.Updating,
            healthVisualState(DataSourcesUiState(isRefreshing = true, health = summary))
        )
    }

    private fun summary(
        availability: AndroidHealthAvailability = AndroidHealthAvailability.Available,
        permission: AndroidHealthPermissionState = AndroidHealthPermissionState.Granted,
        missingPermissionCount: Int = 0,
        updatedAt: Instant? = Instant.parse("2026-07-14T00:00:00Z"),
        error: String? = null
    ) = AndroidHealthSummary(
        availability = availability,
        permissionState = permission,
        missingPermissionCount = missingPermissionCount,
        updatedAt = updatedAt,
        error = error
    )
}
