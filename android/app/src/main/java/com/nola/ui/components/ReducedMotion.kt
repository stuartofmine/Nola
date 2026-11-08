package com.nola.ui.components

import android.content.ContentResolver
import android.database.ContentObserver
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext

internal fun animatorDurationScaleDisablesMotion(scale: Float): Boolean = scale <= 0f

private class ReducedMotionMonitor(
    private val resolver: ContentResolver,
) {
    var enabled by mutableStateOf(readSetting())
        private set

    private val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
        override fun onChange(selfChange: Boolean) {
            enabled = readSetting()
        }
    }

    init {
        runCatching {
            resolver.registerContentObserver(
                Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE),
                false,
                observer,
            )
        }
        // Close the small gap between the initial read and observer registration.
        enabled = readSetting()
    }

    private fun readSetting(): Boolean = runCatching {
        animatorDurationScaleDisablesMotion(
            Settings.Global.getFloat(
                resolver,
                Settings.Global.ANIMATOR_DURATION_SCALE,
                1f,
            ),
        )
    }.getOrDefault(false)
}

private object ReducedMotionMonitors {
    @Volatile
    private var processMonitor: ReducedMotionMonitor? = null

    fun get(resolver: ContentResolver): ReducedMotionMonitor =
        processMonitor ?: synchronized(this) {
            processMonitor ?: ReducedMotionMonitor(resolver).also { processMonitor = it }
        }
}

@Composable
internal fun rememberReduceMotionEnabled(): Boolean {
    val applicationContext = LocalContext.current.applicationContext
    val monitor = remember(applicationContext) {
        ReducedMotionMonitors.get(applicationContext.contentResolver)
    }
    return monitor.enabled
}
