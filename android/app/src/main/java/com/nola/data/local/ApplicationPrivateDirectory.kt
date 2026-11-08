// SPDX-License-Identifier: Apache-2.0
package com.nola.data.local

import android.os.Process
import android.system.Os
import android.system.OsConstants
import java.io.File

/** Tighten the existing app-owned directory, never relocate or rewrite its data. */
internal fun secureApplicationDirectory(directory: File, applicationDirectory: File) {
    val expectedParent = applicationDirectory.canonicalFile
    val metadata = Os.lstat(directory.absolutePath)
    check(OsConstants.S_ISDIR(metadata.st_mode) && metadata.st_uid == Process.myUid()) {
        "Application directory is not an owned real directory"
    }
    check(directory.canonicalFile.parentFile == expectedParent &&
        Os.stat(expectedParent.path).st_uid == Process.myUid()) {
        "Application directory escaped its owner"
    }
    val descriptor = Os.open(directory.absolutePath,
        OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
    try {
        val opened = Os.fstat(descriptor)
        check(OsConstants.S_ISDIR(opened.st_mode) && opened.st_uid == Process.myUid() && opened.st_dev == metadata.st_dev && opened.st_ino == metadata.st_ino) {
            "Application directory changed during validation"
        }
        Os.fchmod(descriptor, 448) // 0700; Android creates no_backup as 0771.
        check(Os.fstat(descriptor).st_mode and 511 == 448) { "Application directory is not private" }
    } finally {
        Os.close(descriptor)
    }
}
