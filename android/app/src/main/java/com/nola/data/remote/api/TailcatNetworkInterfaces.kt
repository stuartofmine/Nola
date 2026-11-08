package com.nola.data.remote.api

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkRequest
import android.net.NetworkCapabilities
import android.os.Build
import java.io.IOException
import java.net.NetworkInterface
import kotlinx.serialization.json.*

/** Platform enumeration only: no socket probes, routes, VPN, or stored identity.
 * ACCESS_NETWORK_STATE is already declared by the app. Registration belongs to
 * the foreground lifecycle; each connection also obtains a fresh snapshot. */
internal class TailcatNetworkInterfaces(context: Context) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private var callback: ConnectivityManager.NetworkCallback? = null

    @Synchronized fun start(changed: () -> Unit) {
        if (callback != null) return
        val observer = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = changed()
            override fun onLost(network: Network) = changed()
            override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) = changed()
        }
        try {
            // Observe restricted and VPN networks too. On API 26–29 remove
            // the three default filters; newer systems expose clearCapabilities.
            val request = NetworkRequest.Builder().apply {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    clearCapabilities()
                } else {
                    removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_RESTRICTED)
                    removeCapability(NetworkCapabilities.NET_CAPABILITY_TRUSTED)
                    removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
                }
            }.build()
            connectivity.registerNetworkCallback(request, observer)
            callback = observer
        } catch (_: Exception) {
            throw IOException("Android network observation unavailable")
        }
    }

    @Synchronized fun stop() {
        val observer = callback ?: return
        connectivity.unregisterNetworkCallback(observer)
        callback = null
    }

    fun snapshot(): String = try {
        val interfaces = NetworkInterface.getNetworkInterfaces()?.toList()
            ?: throw IOException("Android network interfaces unavailable")
        buildJsonArray {
            interfaces.sortedBy { it.index }.forEach { item ->
                add(buildJsonObject {
                    put("name", item.name)
                    put("index", item.index)
                    put("mtu", item.mtu)
                    put("up", item.isUp)
                    put("loopback", item.isLoopback)
                    put("multicast", item.supportsMulticast())
                    put("pointToPoint", item.isPointToPoint)
                    putJsonArray("addresses") {
                        item.interfaceAddresses.mapNotNull { address ->
                            address.address?.hostAddress?.substringBefore('%')?.let { "$it/${address.networkPrefixLength}" }
                        }.sorted().forEach { add(it) }
                    }
                })
            }
        }.toString()
    } catch (_: Exception) {
        // Neither interface addresses nor arbitrary platform errors reach logs.
        throw IOException("Android network interfaces unavailable")
    }
}
