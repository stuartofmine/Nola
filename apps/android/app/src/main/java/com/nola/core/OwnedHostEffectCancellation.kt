package com.nola.core

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.async
import kotlinx.coroutines.channels.ReceiveChannel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.withContext

/** Request physical stop immediately, commit the exact Core interrupt, then
 * await real teardown. Cancelling a coroutine does not prove a tool did not
 * commit an external side effect. The caller retains that uncertainty. */
internal suspend fun <T> awaitOwnedHostEffect(
    cancellation: ReceiveChannel<String>,
    interrupt: suspend (String) -> T,
    operation: suspend () -> T,
): T = coroutineScope {
    val effect = async { operation() }
    try {
        select {
            effect.onAwait { it }
            cancellation.onReceive { reason ->
                effect.cancel(CancellationException("Host effect stop requested: $reason"))
                val interrupted = withContext(NonCancellable) { interrupt(reason) }
                effect.join()
                interrupted
            }
        }
    } catch (cancelled: CancellationException) {
        effect.cancel(cancelled)
        // Losing a collector/worker is transport teardown, not a user Stop.
        // Keep the durable Run and reconcile this delivery on recovery.
        throw cancelled
    }
}

internal class HostEffectCancellationUncertain(val effectId: String) : IllegalStateException(
    "Effect $effectId was interrupted without proof of its external outcome; exact recovery is required",
)
