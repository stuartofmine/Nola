package com.nola.core

import android.os.Process
import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import java.io.FileInputStream
import java.util.UUID
import kotlinx.serialization.json.*

/** OS facts only. Core owns worker encoding, PID-reuse comparison and cancellation policy. */
internal object AndroidHostProcessObservation {
    private fun read(path: String): String {
        val descriptor = Os.open(path, OsConstants.O_RDONLY, 0)
        return FileInputStream(descriptor).bufferedReader().use { it.readText() }.trim()
    }

    fun observe(pid: Int): JsonObject {
        fun unknown() = buildJsonObject { put("state", "unknown"); put("pid", pid) }
        if (pid <= 0) return unknown()
        val boot = try { read("/proc/sys/kernel/random/boot_id") } catch (_: Exception) { return unknown() }
        val stat = try { read("/proc/$pid/stat") } catch (error: ErrnoException) {
            return if (error.errno == OsConstants.ENOENT || error.errno == OsConstants.ESRCH)
                buildJsonObject { put("state", "gone"); put("pid", pid); put("boot_id", boot) }
            else unknown()
        } catch (_: Exception) { return unknown() }
        val start = parseStartIdentity(stat) ?: return unknown()
        return buildJsonObject {
            put("state", "running"); put("pid", pid); put("boot_id", boot); put("start_id", start)
        }
    }

    fun currentWorkerIdentity(): JsonObject {
        val observed = observe(Process.myPid())
        val nonce = UUID.randomUUID().toString().lowercase()
        return if (observed["state"]?.jsonPrimitive?.content == "running") {
            JsonObject((observed - "state") + mapOf("kind" to JsonPrimitive("process"), "nonce" to JsonPrimitive(nonce)))
        } else buildJsonObject {
            put("kind", "unobservable"); put("nonce", nonce)
        }
    }
}

internal fun parseStartIdentity(stat: String): String? {
    val close = stat.lastIndexOf(')')
    if (close < 0) return null
    // /proc/PID/stat field 22; after the parenthesized comm, state is field 3.
    return stat.substring(close + 1).trim().split(Regex("\\s+"))
        .getOrNull(19)?.takeIf { it.toULongOrNull()?.let { value -> value > 0uL } == true }
}
