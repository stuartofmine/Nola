package com.nola.ui.navigation

import org.junit.Assert.assertEquals
import org.junit.Test

class NolaNavigationLayoutTest {
    @Test
    fun phoneWidthsUseBottomNavigation() {
        assertEquals(NolaNavigationLayout.Bottom, nolaNavigationLayout(0f))
        assertEquals(NolaNavigationLayout.Bottom, nolaNavigationLayout(679.99f))
    }

    @Test
    fun tabletWidthsUseCompactSideNavigation() {
        assertEquals(NolaNavigationLayout.CompactSide, nolaNavigationLayout(680f))
        assertEquals(NolaNavigationLayout.CompactSide, nolaNavigationLayout(1179.99f))
    }

    @Test
    fun desktopWidthsUseExpandedSideNavigation() {
        assertEquals(NolaNavigationLayout.ExpandedSide, nolaNavigationLayout(1180f))
        assertEquals(NolaNavigationLayout.ExpandedSide, nolaNavigationLayout(1600f))
    }
}
