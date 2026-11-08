package com.nola.ui.screen.models

import android.content.Context
import com.nola.R
import com.nola.data.remote.dto.AgentSettingsResponse
import com.nola.data.remote.dto.HOST_PERMISSION_PROTOCOL_VERSION

internal enum class WebHostVerificationFailure {
    UnsupportedProtocol,
    IdentityChanged
}

internal class WebHostVerificationException(
    val failure: WebHostVerificationFailure
) : IllegalStateException()

internal suspend fun verifyWebHostConnection(
    expectedHostIdentity: String,
    fetchSettings: suspend () -> AgentSettingsResponse
): String {
    val identity = fetchSettings().hostIdentity
    if (
        identity == null ||
        identity.id.isBlank() ||
        identity.permissionProtocolVersion != HOST_PERMISSION_PROTOCOL_VERSION
    ) {
        throw WebHostVerificationException(WebHostVerificationFailure.UnsupportedProtocol)
    }
    if (expectedHostIdentity.isNotBlank() && expectedHostIdentity != identity.id) {
        throw WebHostVerificationException(WebHostVerificationFailure.IdentityChanged)
    }
    return identity.id
}

internal fun webHostConnectionMessage(context: Context, error: Throwable): String {
    val raw = error.message.orEmpty()
    val normalized = raw.lowercase()
    return when {
        error is WebHostVerificationException && error.failure == WebHostVerificationFailure.UnsupportedProtocol ->
            context.getString(R.string.models_error_host_protocol)
        error is WebHostVerificationException && error.failure == WebHostVerificationFailure.IdentityChanged ->
            context.getString(R.string.models_error_host_identity_changed)
        raw == context.getString(R.string.models_error_host_pairing_failed) -> raw
        error is java.net.SocketTimeoutException || normalized.contains("timed out") ->
            context.getString(R.string.models_error_host_timeout)
        error is java.net.ConnectException ||
            error is java.net.UnknownHostException ||
            normalized.contains("failed to connect") ||
            normalized.contains("connection refused") ->
            context.getString(R.string.models_error_host_unreachable)
        normalized.contains("401") || normalized.contains("403") || normalized.contains("unauthorized") ->
            context.getString(R.string.models_error_host_unauthorized)
        else -> context.getString(R.string.models_error_host_generic)
    }
}
