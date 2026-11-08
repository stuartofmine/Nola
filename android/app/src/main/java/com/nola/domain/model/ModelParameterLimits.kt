package com.nola.domain.model

/** Product configuration bounds; actual provider/engine limits are separate. */
object ModelParameterLimits {
    val outputTokens = 64..1_000_000
    val remoteContextTokens = 512..4_000_000

    fun validatedOutputTokens(value: Int): Int {
        require(value in outputTokens) { "Output token limit must be in $outputTokens" }
        return value
    }
}
