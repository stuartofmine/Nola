package com.nola.core

import org.junit.Assert.*
import org.junit.Test

class AndroidHostProcessObservationTest {
    @Test fun procStartIdentityUsesKernelFieldAndHandlesParenthesesInNames() {
        val fields = MutableList(22) { "0" }.apply { this[0] = "S"; this[19] = "123456" }
        assertEquals("123456", parseStartIdentity("42 (worker (name)) " + fields.joinToString(" ")))
        assertNull(parseStartIdentity("42 (worker) S 1"))
        fields[19] = "0"
        assertNull(parseStartIdentity("42 (worker) " + fields.joinToString(" ")))
    }
}
