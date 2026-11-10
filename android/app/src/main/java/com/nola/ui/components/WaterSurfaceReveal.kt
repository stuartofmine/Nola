package com.nola.ui.components

/** Drawing only: never delays accepted text, accessibility or layout. */
internal object WaterSurfaceReveal {
    const val duration = 520L
    const val completionDuration = 160L
    const val maximumSegments = 128

    fun remaining(elapsed: Long, position: Float, completionElapsed: Long? = null,
                  precedingElapsed: Long? = null): Float {
        val effective = if (completionElapsed == null) elapsed.toFloat() else {
            val before = (elapsed - completionElapsed).coerceAtLeast(0).toFloat()
            before + (duration - before).coerceAtLeast(0f) *
                (completionElapsed.toFloat() / completionDuration).coerceIn(0f, 1f)
        }
        val progress = (effective / duration).coerceIn(0f, 1f)
        val local = ((progress * 1.34f - position) / .34f).coerceIn(0f, 1f)
        val value = (1f - local) * (1f - local) * (1f - local)
        return if (precedingElapsed == null) value else maxOf(value,
            remaining(precedingElapsed, 1f, completionElapsed))
    }
}
