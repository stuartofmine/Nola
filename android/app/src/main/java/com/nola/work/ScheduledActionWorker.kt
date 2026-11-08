package com.nola.work

import android.content.Context
import androidx.hilt.work.HiltWorker
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import com.nola.R
import com.nola.agent.trace.AgentTraceEvent
import com.nola.core.AgentCoreContextComponent
import com.nola.core.AgentCoreContextComponentSource
import com.nola.core.AndroidAgentPermissionRequest
import com.nola.core.AndroidAgentTurnRequest
import com.nola.core.AndroidAgentTurnService
import com.nola.core.DeviceAgentCoordinatorEvent
import com.nola.core.DeviceAgentInputOrigin
import com.nola.data.local.db.entity.ExecutionResultEntity
import com.nola.data.local.db.entity.NotificationEntity
import com.nola.data.local.db.entity.ScheduledActionEntity
import com.nola.data.repository.AgentTraceRepository
import com.nola.data.repository.AndroidDataSourceReader
import com.nola.data.repository.DataSourceReadResult
import com.nola.data.repository.NotificationRepository
import com.nola.data.repository.ScheduledActionRepository
import com.nola.data.repository.ConversationRepository
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.notifications.LocalNotificationService
import dagger.assisted.Assisted
import dagger.assisted.AssistedInject
import java.time.Instant
import java.util.UUID
import kotlinx.coroutines.coroutineScope
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.boolean

@HiltWorker
@OptIn(ExperimentalSerializationApi::class)
class ScheduledActionWorker @AssistedInject constructor(
    @Assisted context: Context,
    @Assisted workerParams: WorkerParameters,
    private val scheduledActionRepo: ScheduledActionRepository,
    private val notificationRepo: NotificationRepository,
    private val localNotificationService: LocalNotificationService,
    private val dataSourceReader: AndroidDataSourceReader,
    private val agentTurnService: AndroidAgentTurnService,
    private val agentTraceRepository: AgentTraceRepository,
    private val scheduler: ScheduledActionScheduler,
    private val applicationRuntime: AndroidApplicationRuntime,
    private val conversationRepo: ConversationRepository,
) : CoroutineWorker(context, workerParams) {

    private val json = Json {
        explicitNulls = false
        encodeDefaults = true
    }

    override suspend fun doWork(): Result {
        return try {
        val actionId = inputData.getString(KEY_ACTION_ID)
        val actions = if (actionId != null) {
            listOfNotNull(scheduledActionRepo.getById(actionId))
        } else {
            scheduledActionRepo.getAll().filter { it.enabled }
        }
        val scheduledAt = inputData.getLong(KEY_SCHEDULED_AT, 0L)
            .takeIf { it > 0L }
            ?: return Result.failure(workDataOf("error" to "missing_stable_occurrence_time"))
        val providedOccurrenceID = inputData.getString(KEY_OCCURRENCE_ID)
        val outcomes = actions.filter { it.enabled }.map { action ->
            val occurrenceID = providedOccurrenceID
                ?.takeIf { actionId == action.id }
                ?: "${action.id}:$scheduledAt"
            action to executeCanonicalRun(
                action = action,
                occurrenceID = occurrenceID,
                scheduledAt = scheduledAt,
            )
        }
        if (outcomes.any { it.second == ScheduledActionOutcome.Deferred }) {
            // Worker retry means observe/recover the original occurrence, not
            // a new model submission or a synthetic terminal failure.
            return Result.retry()
        }
        outcomes.forEach { (action, outcome) ->
            if (outcome != ScheduledActionOutcome.NeedsAttention) {
                scheduler.scheduleNext(action)
            }
        }
        if (outcomes.any { it.second == ScheduledActionOutcome.NeedsAttention }) {
            Result.failure(workDataOf(KEY_NEEDS_ATTENTION to true))
        } else {
            Result.success()
        }
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            Result.retry()
        }
    }
    private suspend fun executeCanonicalRun(
        action: ScheduledActionEntity,
        occurrenceID: String,
        scheduledAt: Long,
    ): ScheduledActionOutcome = coroutineScope {
        val run = applicationRuntime.findAutomationOccurrence(action.id, occurrenceID) ?: run {
            val definition = applicationRuntime.loadAutomationDefinition(action.id)
                ?: error("Scheduled action has no canonical execution definition")
            applicationRuntime.enqueueAutomationRun(action.id, definition.revision, occurrenceID, "schedule", scheduledAt)
        }
        val observation = applicationRuntime.observeAutomationExecution(run.id)
        val receipt = applicationRuntime.reconcileAutomationExecution(run.id)
        if (receipt != null) return@coroutineScope publishReceipt(action, occurrenceID, run.id, receipt,
            observation["notification_delivered"]?.jsonPrimitive?.boolean == true)
        val binding = observation["binding"]?.takeUnless { it == JsonNull }?.jsonObject
        // A dispatch lease authorizes first admission only. Once bound, the
        // original Run's Host delivery protocol owns recovery, never this lease.
        val definition = if (binding == null) applicationRuntime.loadAutomationDefinition(action.id)
            ?: error("Scheduled action has no canonical execution definition") else null
        val lease = if (binding == null) {
            require(checkNotNull(definition).revision == run.definitionRevision) {
                "Occurrence definition changed before admission; original intent remains pending"
            }
            require(definition.metadata["url"]?.jsonPrimitive?.content == action.url &&
                definition.metadata["format"]?.jsonPrimitive?.content == action.format) {
                "Local source projection differs from the canonical definition; refresh before admission"
            }
            val workerId = "android:${applicationContext.packageName}:$id"
            val witness = com.nola.data.local.AndroidAutomationLease(run.id, run.definitionRevision,
                workerId, UUID.randomUUID().toString())
            val now = System.currentTimeMillis()
            if (!applicationRuntime.claimAutomationRun(run.id, workerId, witness.leaseToken, now, now + LEASE_DURATION_MS)) {
                return@coroutineScope ScheduledActionOutcome.Deferred
            }
            witness
        } else null
        try {
            executeAction(action, occurrenceID, scheduledAt, definition?.spec, lease, observation)
            val committed = applicationRuntime.reconcileAutomationExecution(run.id)
            if (committed == null) ScheduledActionOutcome.Deferred
            else publishReceipt(action, occurrenceID, run.id, committed, false)
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            throw cancelled
        } catch (_: ScheduledActionPermissionSuspended) {
            // The same Run keeps its open permission wait. No terminal result
            // or failure is inferred from the worker's suspended observation.
            ScheduledActionOutcome.NeedsAttention
        } catch (error: Exception) {
            lease?.let { applicationRuntime.observeAutomationWorker(it, error.message ?: error.javaClass.simpleName) }
            ScheduledActionOutcome.Deferred
        }
    }

    private suspend fun publishReceipt(action: ScheduledActionEntity, occurrenceID: String,
        occurrenceId: String, receipt: JsonObject, delivered: Boolean): ScheduledActionOutcome {
        val terminal = receipt.getValue("terminal").jsonObject
        val identity = terminal.getValue("identity").jsonObject
        val binding = receipt.getValue("binding").jsonObject
        require(binding.getValue("occurrence_id").jsonPrimitive.content == occurrenceId)
        val session = binding.getValue("conversation_id").jsonPrimitive.content
        require(identity.getValue("conversation_id").jsonPrimitive.content == session &&
            identity.getValue("run_id") == binding.getValue("run_id"))
        val kind = terminal.getValue("result").jsonObject.getValue("value").jsonObject.getValue("kind").jsonPrimitive.content
        val outcome = when (kind) {
            "answered", "action_completed" -> ScheduledActionOutcome.Success
            "incomplete", "blocked" -> ScheduledActionOutcome.NeedsAttention
            "delegation_accepted", "failed", "cancelled" -> ScheduledActionOutcome.AlreadyHandled
            else -> error("Unknown canonical automation result")
        }
        val text = com.nola.domain.projection.CanonicalConversationProjector
            .project(conversationRepo.getProjectionWindow(session, null).projection)
            .filter { it.id == identity.getValue("assistant_message_id").jsonPrimitive.content &&
                it.role == com.nola.domain.model.ChatRole.assistant }.joinToString("\n\n") { it.text }
        val existing = scheduledActionRepo.getResults(action.id).firstOrNull { it.id == "scheduled-result:$occurrenceID" }
            ?: ExecutionResultEntity(id = "scheduled-result:$occurrenceID", actionId = action.id, sourceName = action.name)
        scheduledActionRepo.recordResult(existing.copy(status = kind, llmSummaryText = text.takeIf(String::isNotBlank)))
        if (!delivered) {
            notifyResult(action, text.ifBlank { kind }, outcome == ScheduledActionOutcome.Success,
                receipt.getValue("notification_id").jsonPrimitive.content)
            applicationRuntime.acknowledgeAutomationNotification(occurrenceId, receipt.getValue("notification_id").jsonPrimitive.content)
        }
        if (action.scheduleFrequency == "none") scheduledActionRepo.setEnabled(action.id, false)
        return outcome
    }

    private suspend fun executeAction(
        action: ScheduledActionEntity,
        occurrenceID: String,
        scheduledAt: Long,
        spec: com.nola.data.local.AndroidAutomationSpec?,
        lease: com.nola.data.local.AndroidAutomationLease?,
        observation: JsonObject,
    ) {
        val resultId = "scheduled-result:$occurrenceID"
        val identity = observation.getValue("identity").jsonObject
        val sessionId = identity.getValue("conversation_id").jsonPrimitive.content
        val binding = observation["binding"]?.takeUnless { it == JsonNull }?.jsonObject
        val recovering = binding != null
        val runningResult = ExecutionResultEntity(
            id = resultId,
            actionId = action.id,
            sourceName = action.name,
            sourceUrl = action.url,
            sourceFormat = action.format,
            status = "running",
            scheduledAt = scheduledAt,
            createdAt = scheduledAt,
        )
        scheduledActionRepo.recordResult(runningResult)

        val traceEvents = mutableListOf<AgentTraceEvent>()
        var journalText = ""
        run {
            val prompt = if (recovering) "" else checkNotNull(spec).prompt
            val source = if (recovering) null else {
                require(checkNotNull(spec).skillRefs.isEmpty()) { "Scheduled Android Skill execution is not configured" }
                require(spec.contextRefs == listOf("web_source" to "scheduled_source:${action.id}"))
                dataSourceReader.read(action.toDataSourceConfig())
            }
            val toolPlan = agentTurnService.toolPlan(
                sessionId = sessionId,
                resultEgressAuthorized = true,
                // Recovery revalidates the original invocation before dispatch;
                // no current definition/model can replace the original Run.
                permissionMode = com.nola.core.AndroidAgentPermissionMode.fromWireValue(spec?.permissionProfile),
            )
            val admissionSource = if (recovering) null else applicationRuntime.loadConversation(sessionId)
            val descriptor = if (recovering) com.nola.core.loadAndroidRecoveryDescriptor(sessionId, action.name,
                binding!!.getValue("run_id").jsonPrimitive.content, applicationRuntime::executeQuery)
            else com.nola.core.AndroidCanonicalTurnDescriptor(
                conversationId = sessionId, conversationTitle = action.name,
                submissionId = identity.getValue("submission_id").jsonPrimitive.content,
                userMessageId = identity.getValue("user_message_id").jsonPrimitive.content,
                assistantMessageId = identity.getValue("assistant_message_id").jsonPrimitive.content,
                userText = prompt, userContentJson = null, automationLease = lease, automationSpec = spec,
                expectedConfigurationRevision = admissionSource?.configurationRevision,
                expectedParentMessageId = admissionSource?.messages?.lastOrNull()?.id,
            )
            val turn = agentTurnService.start(
                AndroidAgentTurnRequest(
                    model = if (recovering) null else agentTurnService.captureActiveModel(),
                    sessionId = sessionId,
                    input = prompt,
                    tools = toolPlan.tools,
                    toolPlan = toolPlan,
                    inputOrigin = DeviceAgentInputOrigin.SCHEDULED_ACTION,
                    turnContextComponents = source?.let { listOf(scheduledActionReferenceComponent(action, it, resultId)) } ?: emptyList(),
                    canonicalTurn = descriptor,
                    recoverExistingRun = recovering,
                    resultEgressAuthorized = true,
                ),
            )
            turn.events.collect { event ->
                when (event) {
                    is DeviceAgentCoordinatorEvent.JournalPublications -> {
                        val window = conversationRepo.getProjectionWindow(sessionId, null)
                        journalText = com.nola.domain.projection.CanonicalConversationProjector.project(window.projection)
                            .filter { it.role == com.nola.domain.model.ChatRole.assistant }
                            .joinToString("\n\n") { it.text }
                        scheduledActionRepo.recordResult(runningResult.copy(
                            fetchedAt = source?.contract?.observedAt,
                            llmSummaryText = journalText.takeIf(String::isNotBlank),
                            eventsJson = executionJournalJson(sessionId = sessionId, state = "running", traceEvents = traceEvents),
                        ))
                    }
                    is DeviceAgentCoordinatorEvent.Trace -> {
                        traceEvents += event.event
                        agentTraceRepository.append(event.event)
                        scheduledActionRepo.recordResult(
                            runningResult.copy(
                                fetchedAt = source?.contract?.observedAt,
                                llmSummaryText = journalText.takeIf(String::isNotBlank),
                                eventsJson = executionJournalJson(
                                    sessionId = sessionId,
                                    state = "running",
                                    traceEvents = traceEvents,
                                ),
                            ),
                        )
                    }
                    is DeviceAgentCoordinatorEvent.PermissionRequested -> {
                        scheduledActionRepo.recordResult(
                            runningResult.copy(
                                status = "needs_attention",
                                fetchedAt = source?.contract?.observedAt,
                                llmSummaryText = journalText.takeIf(String::isNotBlank),
                                failureReason = "permission_required",
                                eventsJson = executionJournalJson(
                                    sessionId = sessionId,
                                    state = "needs_attention",
                                    traceEvents = traceEvents,
                                    pendingPermission = event.request,
                                ),
                            ),
                        )
                        throw ScheduledActionPermissionSuspended(event.request)
                    }
                    is DeviceAgentCoordinatorEvent.Completed -> Unit
                    else -> Unit
                }
            }
        }
    }

    private suspend fun notifyResult(
        action: ScheduledActionEntity,
        detail: String,
        succeeded: Boolean,
        notificationID: String,
    ) {
        val title = applicationContext.getString(
            if (succeeded) R.string.scheduled_notification_completed
            else R.string.scheduled_notification_failed,
        )
        val body = "${action.name}: ${detail.take(160)}"
        notificationRepo.add(
            NotificationEntity(
                id = notificationID,
                title = title,
                body = body,
                source = "scheduled",
                notificationType = "scheduled_action",
                dedupeKey = notificationID,
            ),
        )
        localNotificationService.showScheduledActionResult(
            title = title,
            body = body,
            notificationId = notificationID.hashCode(),
        )
    }

    private fun executionJournalJson(
        sessionId: String,
        state: String,
        traceEvents: List<AgentTraceEvent>,
        pendingPermission: AndroidAgentPermissionRequest? = null,
    ): String = buildJsonObject {
        put("schema_version", 1)
        put("session_id", sessionId)
        put("state", state)
        put(
            "trace",
            JsonArray(
                traceEvents.map { event ->
                    json.encodeToJsonElement(AgentTraceEvent.serializer(), event)
                },
            ),
        )
        pendingPermission?.let { request ->
            put(
                "pending_permission",
                buildJsonObject {
                    put("request_id", request.requestId)
                    put("call_id", request.callId)
                    put("tool_name", request.toolName)
                    put("capability", request.capability)
                    put("authorization", request.authorization)
                    put("rationale", request.rationale)
                    request.scope?.let { put("scope", it) }
                },
            )
        }
    }.toString()

    companion object {
        const val KEY_ACTION_ID = "action_id"
        const val KEY_OCCURRENCE_ID = "occurrence_id"
        const val KEY_SCHEDULED_AT = "scheduled_at"
        const val KEY_NEEDS_ATTENTION = "needs_attention"
        private const val LEASE_DURATION_MS = 5 * 60_000L
    }
}

internal fun ScheduledActionEntity.toDataSourceConfig(): CustomDataSourceConfig =
    CustomDataSourceConfig(
        id = "scheduled_source:$id",
        name = name,
        location = url,
        maximumAgeMinutes = 60,
    )

internal fun scheduledActionReferenceComponent(
    action: ScheduledActionEntity,
    source: DataSourceReadResult,
    executionId: String,
): AgentCoreContextComponent = AgentCoreContextComponent(
    kind = "reference",
    id = "scheduled_source:${action.id}:$executionId",
    content = JsonPrimitive(source.content),
    source = AgentCoreContextComponentSource(
        platform = "android",
        uri = action.url,
        title = action.name,
        mimeType = scheduledActionMimeType(action.format),
        observedAt = Instant.ofEpochMilli(source.contract.observedAt).toString(),
        authorization = "authorized",
        maximumAgeMinutes = source.contract.maximumAgeMinutes.toLong(),
    ),
)

internal fun scheduledActionMimeType(format: String): String = when (format.trim().lowercase()) {
    "json" -> "application/json"
    "csv" -> "text/csv"
    "rss" -> "application/rss+xml"
    "icalendar", "calendar" -> "text/calendar"
    "html" -> "text/html"
    else -> "text/plain"
}

private enum class ScheduledActionOutcome {
    Success,
    NeedsAttention,
    Deferred,
    AlreadyHandled;
}

private class ScheduledActionPermissionSuspended(
    val request: AndroidAgentPermissionRequest,
) : Exception("Scheduled Agent turn is waiting for permission")
