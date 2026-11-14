package com.nola.ui.screen.chat

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper

/** Call only after a completed OS request, never before the first request. */
internal fun requiresRuntimePermissionSettings(grants: Map<String, Boolean>, canAskAgain: (String) -> Boolean): Boolean =
    grants.any { (permission, granted) -> !granted && !canAskAgain(permission) }

internal fun Context.permissionRecoveryActivity(): Activity? = when (this) {
    is Activity -> this
    is ContextWrapper -> if (baseContext !== this) baseContext.permissionRecoveryActivity() else null
    else -> null
}
