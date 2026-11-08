// SPDX-License-Identifier: Apache-2.0
package com.nola.data.local

import android.system.Os
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import org.junit.Assert.*
import org.junit.Test

class ApplicationPrivateDirectoryTest {
    @Test fun tightensOwnedDirectoryWithoutChangingDataAndRejectsSymlinkOrOtherParent() {
        val base = File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir, "private-${UUID.randomUUID()}").also { it.mkdir() }
        val directory = File(base, "owner").also { it.mkdir() }
        val data = File(directory, "preserved").apply { writeText("unchanged") }
        Os.chmod(directory.path, 505) // 0771, the platform default.
        secureApplicationDirectory(directory, base)
        assertEquals(448, Os.stat(directory.path).st_mode and 511)
        assertEquals("unchanged", data.readText())
        val link = File(base, "link")
        Os.symlink(directory.path, link.path)
        assertThrows(IllegalStateException::class.java) { secureApplicationDirectory(link, base) }
        assertThrows(IllegalStateException::class.java) { secureApplicationDirectory(directory, directory) }
        assertEquals("unchanged", data.readText())
    }
}
