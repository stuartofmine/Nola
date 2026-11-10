package com.nola.ui.screen.chat

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext

/** Creates Android Host adapters at the chat feature composition boundary. */
@Composable
internal fun rememberAndroidAttachmentImporter(): AndroidAttachmentImporter {
    val applicationContext = LocalContext.current.applicationContext
    return remember(applicationContext) {
        AndroidAttachmentImportCoordinator(applicationContext)
    }
}
