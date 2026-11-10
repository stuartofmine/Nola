package com.nola.core

data class AgentExecutionPresentation(
    val title: String,
    val detail: String,
    val iconName: String,
    val keepsDeviceDataLocal: Boolean
)

fun agentExecutionPresentation(
    executionMode: String,
    hostName: String? = null
): AgentExecutionPresentation = when (executionMode) {
    "host_remote" -> AgentExecutionPresentation(
        title = hostName?.trim()?.takeIf { it.isNotEmpty() } ?: "Host",
        detail = "The connected host runs the agent and tools",
        iconName = "computer",
        keepsDeviceDataLocal = false
    )
    "hybrid" -> AgentExecutionPresentation(
        title = "Host + Device",
        detail = "Device access is shared only after approval",
        iconName = "sync",
        keepsDeviceDataLocal = false
    )
    else -> AgentExecutionPresentation(
        title = "On Device",
        detail = "Model and tools stay on this device",
        iconName = "smartphone",
        keepsDeviceDataLocal = true
    )
}

