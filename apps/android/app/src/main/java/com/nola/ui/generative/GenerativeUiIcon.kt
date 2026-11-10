package com.nola.ui.generative

import androidx.compose.ui.res.stringResource
import com.nola.R

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import kotlinx.serialization.json.*

/** The pinned catalog names map to native vector artwork, never textual placeholders. */
internal fun catalogIcon(name: String): ImageVector? = when (name) {
    "accountCircle" -> Icons.Filled.AccountCircle
    "add" -> Icons.Filled.Add
    "arrowBack" -> Icons.Filled.ArrowBack
    "arrowForward" -> Icons.Filled.ArrowForward
    "attachFile" -> Icons.Filled.AttachFile
    "calendarToday" -> Icons.Filled.CalendarToday
    "call" -> Icons.Filled.Call
    "camera" -> Icons.Filled.Camera
    "check" -> Icons.Filled.Check
    "close" -> Icons.Filled.Close
    "delete" -> Icons.Filled.Delete
    "download" -> Icons.Filled.FileDownload
    "edit" -> Icons.Filled.Edit
    "event" -> Icons.Filled.Event
    "error" -> Icons.Filled.Error
    "fastForward" -> Icons.Filled.FastForward
    "favorite" -> Icons.Filled.Favorite
    "favoriteOff" -> Icons.Filled.FavoriteBorder
    "folder" -> Icons.Filled.Folder
    "help" -> Icons.Filled.Help
    "home" -> Icons.Filled.Home
    "info" -> Icons.Filled.Info
    "locationOn" -> Icons.Filled.LocationOn
    "lock" -> Icons.Filled.Lock
    "lockOpen" -> Icons.Filled.LockOpen
    "mail" -> Icons.Filled.Mail
    "menu" -> Icons.Filled.Menu
    "moreVert" -> Icons.Filled.MoreVert
    "moreHoriz" -> Icons.Filled.MoreHoriz
    "notificationsOff" -> Icons.Filled.NotificationsOff
    "notifications" -> Icons.Filled.Notifications
    "pause" -> Icons.Filled.Pause
    "payment" -> Icons.Filled.Payment
    "person" -> Icons.Filled.Person
    "phone" -> Icons.Filled.Phone
    "photo" -> Icons.Filled.Photo
    "play" -> Icons.Filled.PlayArrow
    "print" -> Icons.Filled.Print
    "refresh" -> Icons.Filled.Refresh
    "rewind" -> Icons.Filled.FastRewind
    "search" -> Icons.Filled.Search
    "send" -> Icons.Filled.Send
    "settings" -> Icons.Filled.Settings
    "share" -> Icons.Filled.Share
    "shoppingCart" -> Icons.Filled.ShoppingCart
    "skipNext" -> Icons.Filled.SkipNext
    "skipPrevious" -> Icons.Filled.SkipPrevious
    "star" -> Icons.Filled.Star
    "starHalf" -> Icons.Filled.StarHalf
    "starOff" -> Icons.Filled.StarBorder
    "stop" -> Icons.Filled.Stop
    "upload" -> Icons.Filled.FileUpload
    "visibility" -> Icons.Filled.Visibility
    "visibilityOff" -> Icons.Filled.VisibilityOff
    "volumeDown" -> Icons.Filled.VolumeDown
    "volumeMute" -> Icons.Filled.VolumeMute
    "volumeOff" -> Icons.Filled.VolumeOff
    "volumeUp" -> Icons.Filled.VolumeUp
    "warning" -> Icons.Filled.Warning
    else -> null
}

/** Rendering safety budget only; the shared Core remains the protocol validator. */
internal fun iconPath(data: String): Path? {
    if (data.isBlank() || data.toByteArray(Charsets.UTF_8).size > 65_536 ||
        data.count { it.isLetter() && it != 'e' && it != 'E' } > 4_096) return null
    return runCatching {
        PathParser().parsePathString(data).toPath().also {
            val bounds = it.getBounds()
            require(listOf(bounds.left, bounds.top, bounds.right, bounds.bottom).all { coordinate -> coordinate.isFinite() })
        }
    }.getOrNull()
}

@Composable
internal fun GenerativeUiIcon(name: JsonElement?, description: String) {
    val named = (name as? JsonPrimitive)?.contentOrNull
    val vector = named?.let(::catalogIcon)
    val svg = (name as? JsonObject)?.get("svgPath")?.jsonPrimitive?.contentOrNull
    val path = remember(svg) { svg?.let(::iconPath) }
    val accessibility = description.ifBlank { named ?: stringResource(R.string.ui_icon_label) }
    if (vector != null) Icon(vector, contentDescription = accessibility, modifier = Modifier.size(24.dp))
    else if (path != null) {
        val tint = MaterialTheme.colorScheme.onSurface
        Canvas(Modifier.size(24.dp).semantics { contentDescription = accessibility }) {
            scale(size.width / 24f, size.height / 24f, pivot = androidx.compose.ui.geometry.Offset.Zero) { drawPath(path, tint) }
        }
    } else Text(stringResource(R.string.ui_icon_unavailable), color = MaterialTheme.colorScheme.error)
}
