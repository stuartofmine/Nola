package com.nola.data.remote.api

import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import io.ktor.client.plugins.contentnegotiation.ContentNegotiation
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.logging.LogLevel
import io.ktor.client.plugins.logging.Logging
import io.ktor.serialization.kotlinx.json.json
import java.security.KeyStore
import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.concurrent.ConcurrentHashMap
import javax.inject.Inject
import javax.inject.Singleton
import javax.net.ssl.TrustManagerFactory
import javax.net.ssl.X509TrustManager
import kotlinx.serialization.json.Json

internal fun normalizeCertificateSha256(value: String?): String? {
    val trimmed = value.orEmpty().trim()
    val withoutPrefix = if (trimmed.startsWith("sha256/", ignoreCase = true)) {
        trimmed.drop(7)
    } else {
        trimmed
    }
    val normalized = withoutPrefix
        .filterNot { it == ':' || it.isWhitespace() }
        .lowercase()
    if (normalized.isEmpty()) return ""
    return normalized.takeIf { it.length == 64 && it.all(Char::isHexDigit) }
}

private fun Char.isHexDigit(): Boolean = this in '0'..'9' || this in 'a'..'f' || this in 'A'..'F'

@Singleton
class WebHostHttpClientFactory @Inject constructor(
    private val sharedClient: HttpClient,
    private val tailcat: NolaTailcatTransport? = null
) {
    private val routedSharedClient by lazy { tailcat?.route(sharedClient.config {}) ?: sharedClient }
    private val pinnedClients = ConcurrentHashMap<String, HttpClient>()
    private val controlClients = ConcurrentHashMap<String, HttpClient>()

    fun clientFor(hostBaseUrl: String, certificateSha256: String): HttpClient {
        require(isAllowedWebHostTransport(hostBaseUrl)) {
            "Public Web Host addresses require HTTPS; HTTP is limited to a private local network"
        }
        val pin = normalizeCertificateSha256(certificateSha256)
            ?: throw IllegalArgumentException("Invalid SHA-256 certificate fingerprint")
        if (pin.isEmpty()) return routedSharedClient
        require(hostBaseUrl.trim().startsWith("https://", ignoreCase = true)) {
            "Certificate pinning requires an HTTPS Web Host URL"
        }
        val cacheKey = "${hostBaseUrl.trim().lowercase()}\u0000$pin"
        return pinnedClients.getOrPut(cacheKey) { createPinnedClient(pin) }
    }

    /**
     * Returns a client with an independent CIO engine for control-plane calls.
     *
     * A long-lived SSE response must never own the connection pool needed to
     * cancel that same run. Keeping cancellation, permission and steering
     * requests on a separate engine also makes this invariant hold in tests
     * and in constrained mobile networks where a route may expose only one
     * reusable HTTP/1.1 connection at a time.
     */
    fun controlClientFor(hostBaseUrl: String, certificateSha256: String): HttpClient {
        require(isAllowedWebHostTransport(hostBaseUrl)) {
            "Public Web Host addresses require HTTPS; HTTP is limited to a private local network"
        }
        val pin = normalizeCertificateSha256(certificateSha256)
            ?: throw IllegalArgumentException("Invalid SHA-256 certificate fingerprint")
        if (pin.isNotEmpty()) {
            require(hostBaseUrl.trim().startsWith("https://", ignoreCase = true)) {
                "Certificate pinning requires an HTTPS Web Host URL"
            }
        }
        val cacheKey = "${hostBaseUrl.trim().lowercase()}\u0000$pin"
        return controlClients.getOrPut(cacheKey) { createClient(pin.takeIf(String::isNotEmpty)) }
    }

    private fun createPinnedClient(certificateSha256: String): HttpClient =
        createClient(certificateSha256)

    private fun createClient(certificateSha256: String?): HttpClient = HttpClient(CIO) {
        install(HttpTimeout) {
            requestTimeoutMillis = 60_000
            connectTimeoutMillis = 20_000
            socketTimeoutMillis = 60_000
        }
        if (certificateSha256 != null) {
            engine {
                https {
                    trustManager = PinnedCertificateTrustManager(certificateSha256)
                }
            }
        }
        install(ContentNegotiation) {
            json(Json {
                ignoreUnknownKeys = true
                isLenient = true
                prettyPrint = false
            })
        }
        install(Logging) {
            level = LogLevel.INFO
        }
    }.let { client -> if (certificateSha256 == null) tailcat?.route(client) ?: client else client }
}

private class PinnedCertificateTrustManager(
    expectedSha256: String
) : X509TrustManager {
    private val expected = expectedSha256.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
    private val platformTrustManager: X509TrustManager = TrustManagerFactory
        .getInstance(TrustManagerFactory.getDefaultAlgorithm())
        .apply { init(null as KeyStore?) }
        .trustManagers
        .filterIsInstance<X509TrustManager>()
        .single()

    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        val certificates = chain ?: throw CertificateException("Client certificate chain is empty")
        platformTrustManager.checkClientTrusted(certificates, authType.orEmpty())
    }

    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        val certificates = chain ?: throw CertificateException("Server certificate chain is empty")
        platformTrustManager.checkServerTrusted(certificates, authType.orEmpty())
        val actual = MessageDigest.getInstance("SHA-256").digest(certificates.first().encoded)
        if (!MessageDigest.isEqual(expected, actual)) {
            throw CertificateException("Web Host certificate fingerprint does not match the paired certificate")
        }
    }

    override fun getAcceptedIssuers(): Array<X509Certificate> = platformTrustManager.acceptedIssuers
}
