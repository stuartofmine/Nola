// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.components

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController

/** Observe unclaimed taps from this window without intercepting editing, clicks or scrolls. */
internal fun Modifier.dismissImeOnBlankTap(): Modifier = composed {
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    pointerInput(focus, keyboard) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Final)
            if (down.isConsumed || down.type != PointerType.Touch) return@awaitEachGesture
            while (true) {
                val event = awaitPointerEvent(PointerEventPass.Final)
                val change = event.changes.singleOrNull() ?: break
                if (change.id != down.id || change.isConsumed ||
                    change.uptimeMillis - down.uptimeMillis >= viewConfiguration.longPressTimeoutMillis ||
                    (change.position - down.position).getDistance() > viewConfiguration.touchSlop) break
                if (!change.pressed) {
                    focus.clearFocus()
                    keyboard?.hide()
                    break
                }
            }
        }
    }
}
