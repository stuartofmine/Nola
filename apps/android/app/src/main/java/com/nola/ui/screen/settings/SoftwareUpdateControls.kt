// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.settings

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import com.nola.BuildConfig
import com.nola.R
import com.nola.core.AgentCoreNative
import java.io.File
import java.io.IOException
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

internal fun softwareUpdateTarget(): String = if (BuildConfig.STORE_MANAGED_UPDATES) "android-store" else when (Build.SUPPORTED_ABIS.firstOrNull()) {
    "arm64-v8a" -> "android-aarch64"
    "x86_64" -> "android-x86_64"
    else -> throw IOException("Unsupported update architecture")
}
private suspend fun updateCall(operation: String, releaseId: String? = null, directory: File? = null): JSONObject = withContext(Dispatchers.IO) {
    val request = JSONObject().put("operation", operation).put("target", softwareUpdateTarget())
    releaseId?.let { request.put("expected_release_id", it) }
    directory?.let { request.put("directory", it.absolutePath) }
    val data = AgentCoreNative.nativeSoftwareUpdate(request.toString().toByteArray(Charsets.UTF_8)) ?: throw IOException("Update service unavailable")
    val reply = JSONObject(String(data, Charsets.UTF_8))
    if (!reply.optBoolean("ok")) throw IOException(reply.optJSONObject("error")?.optString("message") ?: "Update check failed")
    reply.getJSONObject("value")
}

/** The signed feed also has to describe this app's actual APK identity. */
internal fun validateDownloadedAndroidUpdate(context: Context, file: File, version: String, build: Long) {
    val info = context.packageManager.getPackageArchiveInfo(file.absolutePath, 0)
        ?: throw IOException("The download is not a readable Android package")
    val code = if (Build.VERSION.SDK_INT >= 28) info.longVersionCode else {
        @Suppress("DEPRECATION")
        info.versionCode.toLong()
    }
    if (info.packageName != context.packageName || info.versionName != version || code != build) {
        throw IOException("The APK package or version does not match this update")
    }
}

@Composable
internal fun SoftwareUpdateControls() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var snapshot by remember { mutableStateOf<JSONObject?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var packageFile by remember { mutableStateOf<File?>(null) }
    val statusText = when (snapshot?.optString("status")) {
        "disabled" -> R.string.software_update_status_disabled
        "up_to_date" -> R.string.software_update_status_up_to_date
        "ahead" -> R.string.software_update_status_ahead
        "available" -> R.string.software_update_status_available
        "unavailable" -> R.string.software_update_status_unavailable
        "incompatible_data" -> R.string.software_update_status_incompatible_data
        else -> R.string.software_update_not_checked
    }
    fun open(url: String) {
        runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
            .onFailure { failure = it.localizedMessage }
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.software_update_title), style = MaterialTheme.typography.titleSmall)
        Text(stringResource(statusText), color = MaterialTheme.colorScheme.onSurfaceVariant)
        snapshot?.optJSONObject("release")?.let { release ->
            Text("${release.getString("version")} (${release.getLong("build")})")
            release.optString("notes").takeIf { it.isNotBlank() }?.let { Text(it) }
            val artifact = release.getJSONObject("artifact")
            if (artifact.getString("kind") == "store") {
                TextButton(enabled = !busy, onClick = { open(artifact.getString("url")) }) { Text(stringResource(R.string.software_update_store)) }
            } else if (!BuildConfig.STORE_MANAGED_UPDATES) {
                TextButton(enabled = !busy && packageFile == null, onClick = {
                    busy = true; failure = null
                    scope.launch {
                        try {
                            val directory = File(context.cacheDir, "software-updates/${UUID.randomUUID()}")
                            val result = updateCall("download", release.getString("release_id"), directory)
                            val file = File(result.getString("path"))
                            withContext(Dispatchers.IO) {
                                validateDownloadedAndroidUpdate(context, file, release.getString("version"), release.getLong("build"))
                            }
                            packageFile = file
                        } catch (cancel: kotlinx.coroutines.CancellationException) { throw cancel }
                        catch (error: Exception) { failure = error.localizedMessage }
                        finally { busy = false }
                    }
                }) { Text(stringResource(R.string.software_update_download)) }
            }
        }
        packageFile?.let { file ->
            Text(stringResource(R.string.software_update_verified))
            TextButton(enabled = !busy, onClick = {
                runCatching {
                    if (!context.packageManager.canRequestPackageInstalls()) {
                        context.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}")))
                    } else {
                        val uri = FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", file)
                        context.startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(uri, "application/vnd.android.package-archive")
                            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
                    }
                }.onFailure { failure = it.localizedMessage }
            }) { Text(stringResource(R.string.software_update_install)) }
            Text(stringResource(R.string.software_update_install_permission), style = MaterialTheme.typography.bodySmall)
        }
        if (busy) CircularProgressIndicator()
        failure?.let { Text(stringResource(R.string.software_update_failed), color = MaterialTheme.colorScheme.error); Text(it, style = MaterialTheme.typography.bodySmall) }
        TextButton(enabled = !busy, onClick = {
            busy = true; failure = null; packageFile = null
            scope.launch {
                try { snapshot = updateCall("check") }
                catch (cancel: kotlinx.coroutines.CancellationException) { throw cancel }
                catch (error: Exception) { failure = error.localizedMessage; snapshot = null }
                finally { busy = false }
            }
        }) { Text(stringResource(R.string.software_update_check)) }
        Text(stringResource(R.string.software_update_data_hint), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
