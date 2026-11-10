package com.nola.ui.screen.models

import android.content.Context
import com.nola.R
import com.nola.data.remote.dto.AgentSettingsResponse
import com.nola.data.remote.dto.HOST_PERMISSION_PROTOCOL_VERSION
import com.nola.data.remote.api.normalizeWebHostBaseUrl
import com.nola.data.remote.api.isAllowedWebHostTransport
import java.net.URI
import kotlinx.serialization.json.*

internal fun sameWebHostOrigin(first: String, second: String): Boolean = runCatching {
    val a = URI(normalizeWebHostBaseUrl(first))
    val b = URI(normalizeWebHostBaseUrl(second))
    fun port(uri: URI) = if (uri.port >= 0) uri.port else if (uri.scheme.equals("https", true)) 443 else 80
    a.host != null && b.host != null && a.scheme.equals(b.scheme, true) && a.host.equals(b.host, true) && port(a) == port(b)
}.getOrDefault(false)

internal data class WebHostConnectionSetup(val origin: String, val hostId: String, val token: String) {
    companion object {
        fun decode(text: String): WebHostConnectionSetup? = runCatching {
            require(text.length <= 32_768)
            val value = Json.parseToJsonElement(text).jsonObject
            fun string(name: String): String {
                val item = value.getValue(name).jsonPrimitive
                require(item.isString)
                return item.content.trim()
            }
            require(string("type") in setOf("nola.server.connection", "server.started"))
            val version = value.getValue("protocol_version").jsonPrimitive
            require(!version.isString && version.intOrNull == 1)
            val origin = string("origin")
            val uri = URI(origin)
            require(uri.scheme?.lowercase() in setOf("http", "https") && isAllowedWebHostTransport(origin))
            require(uri.userInfo == null && uri.query == null && uri.fragment == null && uri.path.orEmpty() in setOf("", "/"))
            val id = string("host_id")
            val token = string("access_token")
            require(id.isNotEmpty() && token.isNotEmpty())
            WebHostConnectionSetup(origin, id, token)
        }.getOrNull()
    }
}

internal enum class WebHostVerificationFailure {
    UnsupportedProtocol,
    IdentityChanged
}

internal class WebHostVerificationException(
    val failure: WebHostVerificationFailure
) : IllegalStateException()

internal suspend fun verifyWebHostConnection(
    expectedHostIdentity: String,
    importedHostIdentity: String? = null,
    fetchSettings: suspend () -> AgentSettingsResponse
): String {
    val identity = fetchSettings().hostIdentity
    if (
        identity == null ||
        identity.id.length !in 16..128 || identity.id.any { !it.isLetterOrDigit() && it != '-' && it != '_' } ||
        identity.permissionProtocolVersion != HOST_PERMISSION_PROTOCOL_VERSION
    ) {
        throw WebHostVerificationException(WebHostVerificationFailure.UnsupportedProtocol)
    }
    if ((expectedHostIdentity.isNotBlank() && expectedHostIdentity != identity.id) ||
        (importedHostIdentity != null && importedHostIdentity != identity.id)) {
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
