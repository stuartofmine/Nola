package com.nola.ui.screen.chat

/** Layout and generation transitions cannot manufacture a user's follow intent. */
internal fun followAfterViewportChange(
    followsLatest: Boolean,
    userDragging: Boolean,
    movedTowardHistory: Boolean,
    atLatest: Boolean,
    reviewingPermission: Boolean,
): Boolean = when {
    !userDragging -> followsLatest
    movedTowardHistory -> false
    atLatest && !reviewingPermission -> true
    else -> followsLatest
}
