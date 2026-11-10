package com.nola.ui.components

import org.junit.Assert.assertFalse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ReducedMotionTest {
    @Test
    fun disabledAnimatorScaleSuppressesOptionalMotion() {
        assertTrue(animatorDurationScaleDisablesMotion(0f))
        assertTrue(animatorDurationScaleDisablesMotion(-1f))
    }

    @Test
    fun positiveAnimatorScaleKeepsOptionalMotionEnabled() {
        assertFalse(animatorDurationScaleDisablesMotion(0.5f))
        assertFalse(animatorDurationScaleDisablesMotion(1f))
        assertFalse(animatorDurationScaleDisablesMotion(Float.NaN))
    }

    @Test
    fun sharedMotionWindowKeepsStreamingFollowBounded() {
        assertEquals(180, NolaMotion.disclosure)
        assertEquals(120, NolaMotion.streamReveal)
        assertEquals(320, NolaMotion.maximumVisualBacklog)
        assertTrue(NolaMotion.followPasses * NolaMotion.followInterval <= NolaMotion.maximumVisualBacklog)
        assertTrue(
            NolaMotion.reducedMotionFollowPasses * NolaMotion.reducedMotionFollowInterval <
                NolaMotion.followPasses * NolaMotion.followInterval
        )
    }

    @Test
    fun controlStateMotionIsShorterThanStructuralMotion() {
        assertTrue(NolaMotion.controlState <= NolaMotion.base)
        assertTrue(NolaMotion.controlState <= 200)
    }
}
