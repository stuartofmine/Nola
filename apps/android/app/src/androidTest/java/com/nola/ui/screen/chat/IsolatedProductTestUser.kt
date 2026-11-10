// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.content.Context
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import java.io.File
import java.util.UUID

/** Destructive setup needs a dedicated secondary user or an explicitly owned disposable AVD. */
internal fun requireIsolatedProductTestUser(context: Context) {
    val arguments = InstrumentationRegistry.getArguments()
    val expected = arguments.getString("isolatedUserId")?.toIntOrNull()
    val actual = context.applicationInfo.uid / 100_000
    if (expected != null && expected > 0) {
        check(actual == expected) { "Refusing product test setup: actual Android user $actual differs from isolatedUserId $expected" }
        return
    }
    // Ordinary primary users remain forbidden. Only the trusted test host can
    // stage this per-run marker inside the fresh AVD's private app sandbox.
    check(expected == 0 && actual == 0 && Build.HARDWARE in setOf("ranchu", "goldfish")) {
        "Pass a disposable secondary user, or an explicitly owned emulator run"
    }
    val run = checkNotNull(arguments.getString("isolatedAvdRunId")) { "Missing owned-AVD run UUID" }
    UUID.fromString(run)
    val avd = checkNotNull(arguments.getString("isolatedAvdName")) { "Missing owned-AVD name" }
    val markerFile = File(context.filesDir, "nola-owned-test-avd.json")
    check(markerFile.isFile) { "Refusing primary-user setup without an owned-AVD marker" }
    val marker = JSONObject(markerFile.readText())
    check(marker.getBoolean("disposable") && marker.getString("run_uuid") == run && marker.getString("avd_name") == avd && marker.getInt("user_id") == actual) {
        "Owned-AVD marker does not match this test invocation"
    }
}
