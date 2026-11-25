// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.settings

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

internal data class SoftwareUpdateArtifact(
    val kind: String,
    val url: String?,
    val sha256: String? = null,
)

internal data class SoftwareUpdateRelease(
    val releaseId: String,
    val version: String,
    val build: Long,
    val notes: String?,
    val artifact: SoftwareUpdateArtifact,
)

internal data class SoftwareUpdateSnapshot(
    val status: String?,
    val release: SoftwareUpdateRelease?,
)

private fun JsonObject.nonBlankString(key: String): String? =
    (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull?.takeIf { it.isNotBlank() }

internal fun parseSoftwareUpdateSnapshot(value: JsonObject): SoftwareUpdateSnapshot {
    val status = value.nonBlankString("status")
    val releaseObj = value["release"] as? JsonObject
    val release = releaseObj?.let { obj ->
        val version = obj.nonBlankString("version")
            ?: throw IOException("Update metadata is missing release version")
        val build = (obj["build"] as? JsonPrimitive)?.longOrNull?.takeIf { it >= 0L }
            ?: throw IOException("Update metadata is missing release build")
        val artifactObj = obj["artifact"] as? JsonObject
            ?: throw IOException("Update metadata is missing release artifact")
        val kind = artifactObj.nonBlankString("kind")
            ?: throw IOException("Update metadata is missing artifact kind")
        val url = artifactObj.nonBlankString("url")
        val sha256 = artifactObj.nonBlankString("sha256")
        if (kind == "store" && url == null) {
            throw IOException("Update metadata is missing store URL")
        }
        val releaseId = obj.nonBlankString("release_id")
        if (kind != "store" && releaseId == null) {
            throw IOException("Update metadata is missing release ID")
        }
        val notes = obj.nonBlankString("notes")
        SoftwareUpdateRelease(
            releaseId = releaseId.orEmpty(),
            version = version,
            build = build,
            notes = notes,
            artifact = SoftwareUpdateArtifact(kind = kind, url = url, sha256 = sha256),
        )
    }
    return SoftwareUpdateSnapshot(status = status, release = release)
}

internal fun softwareUpdateTarget(context: Context? = null): String = if (BuildConfig.STORE_MANAGED_UPDATES) "android-store" else when (Build.SUPPORTED_ABIS.firstOrNull()) {
    "arm64-v8a" -> "android-aarch64"
    "x86_64" -> "android-x86_64"
    else -> throw IOException(context?.getString(R.string.software_update_error_unsupported_arch) ?: "Unsupported update architecture")
}
private suspend fun updateCall(
    context: Context,
    operation: String,
    releaseId: String? = null,
    directory: File? = null,
    url: String? = null,
    sha256: String? = null,
): JsonObject = withContext(Dispatchers.IO) {
    val request = buildJsonObject {
        put("operation", operation)
        put("target", softwareUpdateTarget(context))
        releaseId?.let { put("expected_release_id", it) }
        url?.let { put("url", it) }
        sha256?.let { put("sha256", it) }
        directory?.let {
            put("directory", it.absolutePath)
            put("dest_dir", it.absolutePath)
        }
    }
    val data = AgentCoreNative.nativeSoftwareUpdate(request.toString().toByteArray(Charsets.UTF_8))
        ?: throw IOException(context.getString(R.string.software_update_error_service_unavailable))
    val reply = runCatching { Json.parseToJsonElement(String(data, Charsets.UTF_8)).jsonObject }
        .getOrElse { throw IOException(context.getString(R.string.software_update_error_check_failed), it) }
    if ((reply["ok"] as? JsonPrimitive)?.booleanOrNull != true) {
        val message = (reply["error"] as? JsonObject)?.nonBlankString("message")
            ?: context.getString(R.string.software_update_error_check_failed)
        throw IOException(message)
    }
    reply["value"] as? JsonObject
        ?: throw IOException(context.getString(R.string.software_update_error_check_failed))
}

/** The signed feed also has to describe this app's actual APK identity. */
internal fun validateDownloadedAndroidUpdate(context: Context, file: File, version: String, build: Long) {
    val info = context.packageManager.getPackageArchiveInfo(file.absolutePath, 0)
        ?: throw IOException(context.getString(R.string.software_update_error_unreadable_package))
    val code = if (Build.VERSION.SDK_INT >= 28) info.longVersionCode else {
        @Suppress("DEPRECATION")
        info.versionCode.toLong()
    }
    if (info.packageName != context.packageName || info.versionName != version || code != build) {
        throw IOException(context.getString(R.string.software_update_error_package_mismatch))
    }
}

@Composable
internal fun SoftwareUpdateControls() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var snapshot by remember { mutableStateOf<SoftwareUpdateSnapshot?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var packageFile by remember { mutableStateOf<File?>(null) }
    val statusText = when (snapshot?.status) {
        "disabled", "unconfigured" -> R.string.software_update_status_disabled
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
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(stringResource(R.string.software_update_title), style = MaterialTheme.typography.titleSmall)
                Text(
                    stringResource(statusText),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            TextButton(enabled = !busy, onClick = {
                busy = true; failure = null; packageFile = null
                scope.launch {
                    try { snapshot = parseSoftwareUpdateSnapshot(updateCall(context, "check")) }
                    catch (cancel: kotlinx.coroutines.CancellationException) { throw cancel }
                    catch (error: Exception) { failure = error.localizedMessage; snapshot = null }
                    finally { busy = false }
                }
            }) {
                if (busy) {
                    CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.width(6.dp))
                }
                Text(stringResource(R.string.software_update_check))
            }
        }
        snapshot?.release?.let { release ->
            Text("${release.version} (${release.build})", style = MaterialTheme.typography.bodyMedium)
            release.notes?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
            val artifact = release.artifact
            if (artifact.kind == "store") {
                artifact.url?.let { storeUrl ->
                    TextButton(enabled = !busy, onClick = { open(storeUrl) }) { Text(stringResource(R.string.software_update_store)) }
                }
            } else if (!BuildConfig.STORE_MANAGED_UPDATES) {
                TextButton(enabled = !busy && packageFile == null, onClick = {
                    busy = true; failure = null
                    scope.launch {
                        try {
                            val directory = File(context.cacheDir, "software-updates/${UUID.randomUUID()}")
                            val result = updateCall(
                                context = context,
                                operation = "download",
                                releaseId = release.releaseId,
                                directory = directory,
                                url = release.artifact.url,
                                sha256 = release.artifact.sha256,
                            )
                            val path = result.nonBlankString("path")
                                ?: throw IOException(context.getString(R.string.software_update_error_check_failed))
                            val file = File(path)
                            withContext(Dispatchers.IO) {
                                validateDownloadedAndroidUpdate(context, file, release.version, release.build)
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
            Text(stringResource(R.string.software_update_verified), style = MaterialTheme.typography.bodySmall)
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
            Text(stringResource(R.string.software_update_install_permission), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        failure?.let {
            Text(stringResource(R.string.software_update_failed), color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Text(stringResource(R.string.software_update_data_hint), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
