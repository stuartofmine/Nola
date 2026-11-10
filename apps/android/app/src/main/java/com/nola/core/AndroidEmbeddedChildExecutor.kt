package com.nola.core

import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import java.util.concurrent.ConcurrentHashMap

/** Host adapter only. Rust owns claims, renewal, reduction, recovery and sealing. */
internal object AndroidEmbeddedChildExecutor {
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.IO)
    private val active=ConcurrentHashMap<String,Job>()
    @Volatile var onTaskChanged: (() -> Unit)? = null
    fun isActive(taskId: String): Boolean = synchronized(active) { active[taskId]?.isActive == true }
    fun start(taskId:String,parent:AndroidCanonicalAgentRuntime,
        prepare: (JsonObject) -> AndroidPreparedChildExecutor) {
        synchronized(active) {
            if(active[taskId]?.isActive==true)return
            val job=scope.launch(start=CoroutineStart.LAZY) {
                var handle=0L
                var wasAdmitted=false
                try {
                    val candidate=parent.mobileCommand(buildJsonObject{put("type","load_mobile_task_child_configuration");put("task_id",taskId)}).jsonObject
                    val prepared = prepare(candidate)
                    val worker=AndroidHostProcessObservation.currentWorkerIdentity()
                    val now=System.currentTimeMillis()
                    val admitted=parent.mobileCommand(buildJsonObject{put("type","admit_mobile_task_child");put("input",buildJsonObject{
                        put("task_id",taskId);put("parent_conversation_id",parent.descriptor.conversationId);put("worker",worker)
                        put("now_ms",now);put("lease_until_ms",now+120_000);put("candidate_config",candidate)
                        put("current_tools", JsonArray(prepared.currentTools))
                    })})
                    if(admitted==JsonNull)return@launch
                    wasAdmitted=true
                    onTaskChanged?.invoke()
                    val value=admitted.jsonObject
                    val invocation=value.getValue("invocation").jsonObject
                    val child=parent.embeddedChild(invocation)
                    val start=buildJsonObject{
                        put("task_id",taskId);put("parent_conversation_id",parent.descriptor.conversationId)
                        for(key in listOf("conversation_id","run_id","assistant_message_id"))put(key,invocation.getValue(key))
                        put("attempt",value.getValue("attempt"));put("current",value.getValue("current"));put("worker",worker);put("mailbox_capacity",8)
                        put("cancellation_only", value.getValue("cancellation_only"))
                    }
                    handle=parent.startEmbeddedPump(start)
                    val execute=prepared.bind(child)
                    val state=DeviceAgentCoordinator.RunState(invocation.getValue("core_session_id").jsonPrimitive.content,
                        AndroidAgentExecutionMode.DeviceLocal.wireValue,candidate.getValue("host_catalog").jsonObject.getValue("host_id").jsonPrimitive.content)
                    coroutineScope {
                    val requests=ConcurrentHashMap<String,Pair<JsonObject,Job>>()
                    try {
                    while(currentCoroutineContext().isActive) {
                        val polled=decode(AgentCoreNative.nativeChildPumpPoll(handle,500))
                        if(polled["transport_finished"]?.jsonPrimitive?.boolean==true){
                            check(polled["error"]==null||polled["error"]==JsonNull){polled["error"].toString()}
                            break // Transport completion is not a client-created Task terminal.
                        }
                        val request=polled["request"] as? JsonObject ?: continue
                        val pumpId=polled.getValue("pump_id")
                        val ticket=request.getValue("ticket")
                        val sink=object:AgentCoreTransitionRuntime,AndroidCommittedChildMailbox {
                            override suspend fun commit(response:JsonObject) {
                                val accepted=decode(AgentCoreNative.nativeChildPumpRespond(handle,buildJsonObject{
                                    put("pump_id",pumpId);put("ticket",ticket);put("response",response)
                                }.toString().encodeToByteArray()))
                                check(accepted["committed"]?.jsonPrimitive?.boolean==true){"Child response commit was not confirmed"}
                            }
                            override suspend fun submit(submission:JsonObject):JsonObject=error("Child boundary input must use Application")
                            override suspend fun snapshot():JsonObject=error("Child state belongs to Core")
                            override suspend fun pendingHostRequests():JsonObject=error("Rust is the sole child request pump")
                        }
                        val envelope=request.getValue("envelope").jsonObject
                        val payload=envelope.getValue("payload").jsonObject
                        if(payload["type"]?.jsonPrimitive?.content=="cancel") {
                            val targetId=payload.getValue("target_effect_id").jsonPrimitive.content
                            val target=requests[targetId]
                            if(target!=null) {
                                if(target.first.getValue("payload").jsonObject["type"]?.jsonPrimitive?.content=="execute_tool")
                                    state.cancellationEvidence.outcomeUnknown(target.first)
                                target.second.cancelAndJoin()
                                val kind=target.first.getValue("payload").jsonObject["type"]?.jsonPrimitive?.content
                                if(kind=="model_stream"||kind=="compact"||kind=="request_permission")
                                    state.cancellationEvidence.returnedLocally(target.first,child.descriptor.conversationId)
                            }
                        }
                        val effect=envelope.getValue("effect_id").jsonPrimitive.content
                        check(requests[effect]?.second?.isActive!=true){"Core dispatched an overlapping effect ticket"}
                        state.cancellationEvidence.dispatching(envelope)
                        val job=launch(start=CoroutineStart.LAZY) {
                            execute(envelope,sink,state)
                            state.cancellationEvidence.returnedLocally(envelope,child.descriptor.conversationId)
                        }
                        requests[effect]=envelope to job
                        job.start()
                    }
                    } finally {
                        // Close before coroutineScope joins physical handlers:
                        // a cancelled coroutine may still be blocked in native
                        // respond awaiting a commit ACK. Closing releases that
                        // wait, but does not manufacture cancellation evidence.
                        AgentCoreNative.nativeChildPumpClose(handle)
                    }
                    }
                } catch(error:Exception) {
                    android.util.Log.e("NolaChildHost","Child transport stopped without a fabricated terminal: $taskId",error)
                } finally {
                    if(handle!=0L){AgentCoreNative.nativeChildPumpClose(handle);AgentCoreNative.nativeChildPumpFree(handle)}
                    val completingJob = checkNotNull(currentCoroutineContext()[Job])
                    synchronized(active) { active.remove(taskId, completingJob) }
                    if(wasAdmitted) onTaskChanged?.invoke()
                }
            }
            active[taskId]=job;job.start()
        }
    }
    private fun decode(bytes:ByteArray?):JsonObject {
        val root=Json.parseToJsonElement(checkNotNull(bytes){"Child mailbox returned no receipt"}.decodeToString()).jsonObject
        check(root["ok"]?.jsonPrimitive?.boolean==true){root["error"].toString()}
        return root.getValue("result").jsonObject
    }
}
internal interface AndroidCommittedChildMailbox { suspend fun commit(response:JsonObject) }

/** One live plan feeds both Core's observation check and the physical executor. */
internal data class AndroidPreparedChildExecutor(
    val currentTools: List<JsonElement>,
    val bind: (AndroidCanonicalAgentRuntime) ->
        (suspend (JsonObject, AgentCoreTransitionRuntime, DeviceAgentCoordinator.RunState) -> Unit),
)
