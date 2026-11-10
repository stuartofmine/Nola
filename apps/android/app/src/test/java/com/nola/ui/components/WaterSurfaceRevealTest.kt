package com.nola.ui.components

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class WaterSurfaceRevealTest {
    @Test fun waveTravelsFromHeadToTailWithoutOvershoot() {
        for (elapsed in listOf(-10L, 0L, 40L, 100L, 260L, 450L, 520L, 900L)) {
            val head = WaterSurfaceReveal.remaining(elapsed, 0f)
            val tail = WaterSurfaceReveal.remaining(elapsed, 1f)
            assertTrue(head <= tail)
            assertTrue(head >= 0f && tail <= 1f)
        }
        assertEquals(1f, WaterSurfaceReveal.remaining(0, 0f), .00001f)
        assertEquals(0f, WaterSurfaceReveal.remaining(520, 1f), .00001f)
    }

    @Test fun laterBatchNeverAppearsAheadOfPreviousTail() {
        assertTrue(WaterSurfaceReveal.remaining(200, 0f, precedingElapsed = 250) >=
            WaterSurfaceReveal.remaining(250, 1f))
        assertEquals(0f, WaterSurfaceReveal.remaining(530, 1f, precedingElapsed = 580), .00001f)
    }

    @Test fun completingStreamSettlesWithin160Milliseconds() {
        for (elapsed in listOf(0L, 20L, 100L, 520L)) {
            assertEquals(0f, WaterSurfaceReveal.remaining(elapsed, 1f,
                completionElapsed = 160, precedingElapsed = elapsed + 20), .00001f)
        }
    }
}
