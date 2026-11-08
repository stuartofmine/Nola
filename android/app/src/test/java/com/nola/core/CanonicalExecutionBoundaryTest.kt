package com.nola.core

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CanonicalExecutionBoundaryTest {
    @Test
    fun executionRequiresBothApplicationAbiAndTheShippedCommandProtocol() {
        assertTrue(AgentCoreNative.supportsApplicationContract(1, 54))
        assertFalse(AgentCoreNative.supportsApplicationContract(2, 54))
        assertFalse(AgentCoreNative.supportsApplicationContract(1, 52))
        assertFalse(AgentCoreNative.supportsApplicationContract(1, 53))
        assertFalse(AgentCoreNative.supportsApplicationContract(1, 55))
    }

    @Test
    fun nativeSurfaceHasNoMutableReducerEscapeHatch() {
        val methods = AgentCoreNative::class.java.declaredMethods.map { it.name }
        assertFalse(methods.any { it.startsWith("nativeV2") })
        assertTrue(methods.contains("nativeApplicationSubmit"))
        assertTrue(methods.contains("nativeToolPolicy"))
        assertTrue(methods.contains("nativeIntelligenceAbiVersion"))
        assertFalse(AgentCoreTransitionRuntime::class.java.isAssignableFrom(AgentCoreToolPolicyEvaluator::class.java))
    }
}
