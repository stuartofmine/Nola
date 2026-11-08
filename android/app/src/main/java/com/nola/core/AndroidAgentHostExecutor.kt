@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationManager
import android.net.Uri
import android.provider.CalendarContract
import android.system.Os
import androidx.core.content.ContextCompat
import com.nola.data.local.datastore.MemoryProfileDataStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.time.Instant
import java.time.ZoneId
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

data class AndroidAgentHostCallRequest(
    val callId: String,
    val toolName: String,
    val capability: String,
    val arguments: JsonObject,
    val grant: JsonObject?
)

data class AndroidAgentHostCallResult(
    val callId: String,
    val output: JsonElement,
    val isError: Boolean,
    val artifacts: JsonArray = JsonArray(emptyList())
)

class AndroidAgentHostExecutor(
    private val context: Context,
    private val memoryProfileDataStore: MemoryProfileDataStore,
) {
    internal suspend fun readableMcpServerIds(): Set<String> {
        val principal=memoryProfileDataStore.getOrCreateResourcePrincipalId()
        return memoryProfileDataStore.getSnapshot().mcpServers
            .filter { it.enabled && AndroidManagedResourceAccess.project(it,principal).canReadContent }
            .map { it.serverId }.toSet()
    }
    fun runtimeHandlerCapabilities(): Set<String> = AndroidAgentToolRegistry.capabilities
    fun runtimeHandlerIds(): Set<String> = AndroidAgentToolRegistry.handlerIds

    private fun requireAllowedByPlan(
        request: AndroidAgentHostCallRequest,
        plan: AgentCoreAndroidToolPlan,
    ) {
        require(plan.receiptIsValid()) { "The Core tool plan is expired or invalid" }
        require(plan.allows(request.toolName, request.capability, route = "device")) {
            "Tool execution is not present in the bound Core tool plan"
        }
    }
    companion object {
        private const val MAX_FILE_BYTES = 1_000_000
        private const val MAX_NETWORK_BYTES = 2_000_000
        private const val MAX_SKILL_BYTES = 256_000
    }


    suspend fun execute(
        request: AndroidAgentHostCallRequest,
        toolPlan: AgentCoreAndroidToolPlan,
    ): AndroidAgentHostCallResult =
        runCatching {
            requireAllowedByPlan(request, toolPlan)
            require(!request.toolName.startsWith("host.") && !request.toolName.startsWith("host_")) {
                "Host tools cannot execute in the Android device executor"
            }
            when {
                request.toolName == "skill_invoke" -> invokeSkill(request)
                request.toolName == "memory_search" -> AndroidAgentHostCallResult(
                    callId = request.callId,
                    output = searchMemory(request),
                    isError = false,
                )
                request.capability == "files_read" -> readScopedFile(request)
                request.capability == "files_write" -> writeScopedFile(request)
                request.capability == "clipboard" -> accessClipboard(request)
                request.capability == "location" -> currentLocation(request)
                request.capability == "calendar" -> accessCalendar(request)
                else -> error("Capability ${request.capability} has no Android host executor")
            }
        }.getOrElse { error ->
            if (error is kotlinx.coroutines.CancellationException) throw error
            AndroidAgentHostCallResult(
                callId = request.callId,
                output = buildJsonObject {
                    put("error", error.message ?: "Android host call failed")
                    put("capability", request.capability)
                    put("tool_name", request.toolName)
                },
                isError = true
            )
        }

    private suspend fun searchMemory(request: AndroidAgentHostCallRequest): JsonElement {
        require(request.arguments.keys.all { it == "query" || it == "limit" }) {
            "Personal memory search accepts only query and limit. Local conversation/workspace scopes are unavailable in this tool."
        }
        val query = request.arguments["query"]
            ?.jsonPrimitive
            ?.contentOrNull
            ?.trim()
            ?.takeIf(String::isNotEmpty)
            ?: error("memory_search requires query")
        val limit = request.arguments["limit"]
            ?.jsonPrimitive
            ?.contentOrNull
            ?.toIntOrNull()
            ?.coerceIn(1, 20)
            ?: 8
        val ranked = memoryProfileDataStore.recallProfileMemories(query, 50).map { Triple(0, it.confidence, it) }
        val records = ranked.mapNotNull { (_, _, memory) ->
            val retention = when (memory.retention.trim().lowercase()) {
                "persistent", "permanent" -> AgentCoreMemoryRetention(mode = "permanent")
                "session" -> AgentCoreMemoryRetention(mode = "session")
                "until_superseded" -> AgentCoreMemoryRetention(mode = "until_superseded")
                "expires_at" -> memory.expiresAt?.let {
                    AgentCoreMemoryRetention(
                        mode = "expires_at",
                        expiresAt = Instant.ofEpochMilli(it).toString(),
                    )
                }
                else -> null
            } ?: return@mapNotNull null
            AgentCoreMemoryRecord(
                id = memory.id,
                owner = memory.owner,
                namespace = when (memory.namespace) {
                    "global", "personal" -> AgentCoreMemoryNamespace(kind = "personal")
                    "workspace" -> AgentCoreMemoryNamespace(kind = "workspace")
                    "conversation" -> AgentCoreMemoryNamespace(kind = "conversation")
                    else -> AgentCoreMemoryNamespace(kind = "custom", name = memory.namespace)
                },
                scope = when (memory.scope) {
                    "turn" -> AgentCoreMemoryScope.TURN
                    "conversation" -> AgentCoreMemoryScope.CONVERSATION
                    else -> AgentCoreMemoryScope.GLOBAL
                },
                kind = when (memory.kind) {
                    "preference" -> AgentCoreMemoryKind.PREFERENCE
                    "identity" -> AgentCoreMemoryKind.IDENTITY
                    "decision" -> AgentCoreMemoryKind.DECISION
                    "project" -> AgentCoreMemoryKind.PROJECT
                    "feedback" -> AgentCoreMemoryKind.FEEDBACK
                    "reference" -> AgentCoreMemoryKind.REFERENCE
                    else -> AgentCoreMemoryKind.OTHER
                },
                content = memory.content,
                tags = memory.tags,
                confidence = memory.confidence.toFloat(),
                importance = 0.5f,
                sensitivity = memory.sensitivity,
                retention = retention,
                supersedes = memory.supersedes,
                provenance = AgentCoreMemoryProvenance(
                    kind = when (memory.provenanceKind) {
                        "explicit_user" -> AgentCoreMemoryProvenanceKind.EXPLICIT_USER
                        "agent_inference" -> AgentCoreMemoryProvenanceKind.AGENT_INFERENCE
                        "tool_result" -> AgentCoreMemoryProvenanceKind.TOOL_RESULT
                        "imported" -> AgentCoreMemoryProvenanceKind.IMPORTED
                        "synced" -> AgentCoreMemoryProvenanceKind.SYNCED
                        else -> AgentCoreMemoryProvenanceKind.OTHER
                    },
                    source = memory.sourceReference?.takeIf(String::isNotBlank)
                        ?: "android://memory/${memory.id}",
                    observedAt = memory.observedAt?.let { Instant.ofEpochMilli(it).toString() },
                ),
            )
        }
        val plan = checkNotNull(AgentCoreMemorySelection.select(
            AgentCoreMemorySelectionInput(
                policy = AgentCoreMemoryPolicy(
                    longTermEnabled = true,
                    automaticExtractionEnabled = true,
                    searchLimit = limit,
                    minimumConfidence = 0.5f,
                    maximumAgeDays = 365,
                ),
                records = records,
                mode = AgentCoreMemoryRecallMode.ON_DEMAND,
                allowedNamespaces = records.map { it.namespace }.distinct(),
                includeSessionRetention = false,
                now = Instant.now().toString(),
            )
        )) { "The memory engine is unavailable. Rebuild or update Nola before retrying." }
        return buildJsonObject {
            put("type", "memory_search_result")
            put("query", query)
            put("matches", buildJsonArray {
                plan.selected.forEach { selected ->
                    add(buildJsonObject {
                        put("memory_id", selected.record.id)
                        put("content", selected.record.content)
                        put("kind", selected.record.kind.name.lowercase())
                        put("source", selected.record.provenance.source)
                        put(
                            "requires_current_state_verification",
                            selected.requiresCurrentStateVerification,
                        )
                    })
                }
            })
            put("plan_digest", plan.planDigest)
        }
    }

    private suspend fun invokeSkill(
        request: AndroidAgentHostCallRequest,
    ): AndroidAgentHostCallResult {
        val skillId = request.arguments.text("skill")?.trim().orEmpty()
        require(skillId.isNotEmpty()) { "A Skill invocation requires an exact advertised name" }
        val resourcePrincipalId = memoryProfileDataStore.getOrCreateResourcePrincipalId()
        val matches = memoryProfileDataStore.getSnapshot().uploadedSkills.filter {
            it.enabled && it.name.trim() == skillId && it.content.isNotBlank() &&
                AndroidManagedResourceAccess.canReadContent(it, resourcePrincipalId)
        }
        require(matches.size == 1) { "Unknown, disabled, or ambiguous Skill id: $skillId" }
        val instructions = matches.single().content
        val bytes = instructions.encodeToByteArray()
        require(bytes.size <= MAX_SKILL_BYTES) { "Skill instructions exceed the Android size limit" }
        val digest = MessageDigest.getInstance("SHA-256")
            .digest(bytes)
            .joinToString(separator = "") { byte -> "%02x".format(byte) }
        val version = matches.single().version?.trim()?.takeIf(String::isNotEmpty)
            ?: "sha256-${digest.take(12)}"
        val activation = request.arguments.text("activation")
            ?.trim()
            ?.takeIf { it == "session" }
            ?: "turn"
        val payload = buildJsonObject {
            put("type", "invoked_skill")
            put("skill_id", skillId)
            put("display_name", matches.single().name)
            put("version", version)
            put("instructions_ref", "skill://$skillId/$version/SKILL.md#sha256=$digest")
            put("instructions_digest", digest)
            put("base_directory", "android://skills/$skillId")
            put("mode", "inline")
            put("budget", buildJsonObject {
                put("max_model_turns", 8)
                put("max_tool_calls", 32)
                put("max_elapsed_millis", 300_000)
            })
            put("arguments", request.arguments["args"] ?: JsonObject(emptyMap()))
            put("instructions", instructions)
            put("required_tools", JsonArray(matches.single().requiredTools.map { toolName ->
                buildJsonObject {
                    put("provider", "builtin")
                    put("tool_name", toolName)
                }
            }))
            put("validation_steps", JsonArray(matches.single().validationSteps.map(::JsonPrimitive)))
            put("activation", activation)
        }
        return AndroidAgentHostCallResult(
            callId = request.callId,
            output = payload,
            isError = false,
        )
    }

    private suspend fun readScopedFile(request: AndroidAgentHostCallRequest): AndroidAgentHostCallResult =
        withContext(Dispatchers.IO) {
            val uri = request.scopedUri()
            val stream = context.contentResolver.openInputStream(uri)
                ?: error("Unable to open the granted document")
            val bytes = stream.use { readLimited(it, MAX_FILE_BYTES) }
            AndroidAgentHostCallResult(
                callId = request.callId,
                output = buildJsonObject {
                    put("uri", uri.toString())
                    put("content", bytes.decodeToString())
                    put("bytes", bytes.size)
                },
                isError = false,
                artifacts = androidAgentFileArtifacts(uri.toString(), "read")
            )
        }

    private suspend fun writeScopedFile(request: AndroidAgentHostCallRequest): AndroidAgentHostCallResult =
        withContext(Dispatchers.IO) {
            val uri = request.scopedUri()
            val content = request.arguments.text("content")
                ?: request.arguments.text("text")
                ?: error("A write host call requires content")
            val bytes = content.encodeToByteArray()
            require(bytes.size <= MAX_FILE_BYTES) { "Write exceeds the mobile file limit" }
            val stream = context.contentResolver.openOutputStream(uri, "wt")
                ?: error("Unable to open the granted document for writing")
            stream.use { it.write(bytes) }
            AndroidAgentHostCallResult(
                callId = request.callId,
                output = buildJsonObject {
                    put("uri", uri.toString())
                    put("bytes_written", bytes.size)
                },
                isError = false,
                artifacts = androidAgentFileArtifacts(uri.toString(), "create")
            )
        }

    private suspend fun fetchNetworkResource(request: AndroidAgentHostCallRequest): AndroidAgentHostCallResult =
        withContext(Dispatchers.IO) {
            val raw = request.arguments.text("url") ?: error("A network host call requires url")
            val url = URL(raw)
            require(url.protocol == "http" || url.protocol == "https") {
                "Only HTTP(S) URLs are allowed"
            }
            val connection = (url.openConnection() as HttpURLConnection).apply {
                requestMethod = "GET"
                connectTimeout = 10_000
                readTimeout = 15_000
                instanceFollowRedirects = false
                setRequestProperty("Accept", "application/json, text/plain, text/html;q=0.8, */*;q=0.1")
            }
            try {
                val status = connection.responseCode
                val source = if (status in 200..299) connection.inputStream else connection.errorStream
                val bytes = source?.use { readLimited(it, MAX_NETWORK_BYTES) } ?: ByteArray(0)
                AndroidAgentHostCallResult(
                    callId = request.callId,
                    output = buildJsonObject {
                        put("url", url.toString())
                        put("status", status)
                        put("content_type", connection.contentType.orEmpty())
                        put("body", bytes.decodeToString())
                    },
                    isError = status !in 200..299
                )
            } finally {
                connection.disconnect()
            }
        }

    private suspend fun accessClipboard(request: AndroidAgentHostCallRequest): AndroidAgentHostCallResult =
        withContext(Dispatchers.Main.immediate) {
            val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            val writeText = request.arguments.text("content") ?: request.arguments.text("text")
            if (writeText != null) {
                clipboard.setPrimaryClip(ClipData.newPlainText("Nola", writeText))
                AndroidAgentHostCallResult(
                    callId = request.callId,
                    output = buildJsonObject { put("written", true) },
                    isError = false
                )
            } else {
                val text = clipboard.primaryClip
                    ?.getItemAt(0)
                    ?.coerceToText(context)
                    ?.toString()
                    .orEmpty()
                AndroidAgentHostCallResult(
                    callId = request.callId,
                    output = buildJsonObject { put("content", text) },
                    isError = false
                )
            }
        }

    private suspend fun currentLocation(request: AndroidAgentHostCallRequest): AndroidAgentHostCallResult =
        withContext(Dispatchers.IO) {
            val coarse = ContextCompat.checkSelfPermission(context, android.Manifest.permission.ACCESS_COARSE_LOCATION)
            require(coarse == PackageManager.PERMISSION_GRANTED) {
                "Location permission is not granted"
            }
            val manager = context.getSystemService(Context.LOCATION_SERVICE) as LocationManager
            val location: Location = manager.getProviders(true)
                .mapNotNull { provider -> runCatching { manager.getLastKnownLocation(provider) }.getOrNull() }
                .maxByOrNull { it.time }
                ?: error("No recent device location is available")
            AndroidAgentHostCallResult(
                callId = request.callId,
                output = buildJsonObject {
                    put("latitude", location.latitude)
                    put("longitude", location.longitude)
                    put("horizontal_accuracy_m", location.accuracy.toDouble())
                    put("timestamp_iso", Instant.ofEpochMilli(location.time).toString())
                },
                isError = false
            )
        }

    private suspend fun accessCalendar(request: AndroidAgentHostCallRequest): AndroidAgentHostCallResult =
        withContext(Dispatchers.IO) {
            if (request.toolName == "device_create_calendar_event") {
                createCalendarEvent(request)
            } else {
                listCalendarEvents(request)
            }
        }

    private fun listCalendarEvents(request: AndroidAgentHostCallRequest): AndroidAgentHostCallResult {
        val start = request.arguments.instant("start_iso") ?: Instant.now()
        val end = request.arguments.instant("end_iso") ?: start.plusSeconds(86_400)
        require(end.isAfter(start) && end.epochSecond - start.epochSecond <= 31L * 86_400L) {
            "The calendar time range must be positive and no longer than 31 days"
        }
        val uri = CalendarContract.Instances.CONTENT_URI.buildUpon().also {
            ContentUris.appendId(it, start.toEpochMilli())
            ContentUris.appendId(it, end.toEpochMilli())
        }.build()
        val projection = arrayOf(
            CalendarContract.Instances.EVENT_ID,
            CalendarContract.Instances.TITLE,
            CalendarContract.Instances.BEGIN,
            CalendarContract.Instances.END,
            CalendarContract.Instances.ALL_DAY,
            CalendarContract.Instances.CALENDAR_DISPLAY_NAME
        )
        val events = mutableListOf<JsonElement>()
        context.contentResolver.query(uri, projection, null, null, CalendarContract.Instances.BEGIN + " ASC")?.use { cursor ->
            while (cursor.moveToNext() && events.size < 200) {
                events += buildJsonObject {
                    put("id", cursor.getLong(0).toString())
                    put("title", cursor.getString(1).orEmpty())
                    put("start_iso", Instant.ofEpochMilli(cursor.getLong(2)).toString())
                    put("end_iso", Instant.ofEpochMilli(cursor.getLong(3)).toString())
                    put("all_day", cursor.getInt(4) != 0)
                    put("calendar", cursor.getString(5).orEmpty())
                }
            }
        }
        return AndroidAgentHostCallResult(
            callId = request.callId,
            output = buildJsonObject {
                put("events", kotlinx.serialization.json.JsonArray(events))
                put("truncated", events.size == 200)
            },
            isError = false
        )
    }

    private fun createCalendarEvent(request: AndroidAgentHostCallRequest): AndroidAgentHostCallResult {
        val title = request.arguments.text("title")?.trim().orEmpty()
        require(title.isNotEmpty()) { "A calendar event requires title" }
        val start = request.arguments.instant("start_iso") ?: error("A calendar event requires start_iso")
        val end = request.arguments.instant("end_iso") ?: error("A calendar event requires end_iso")
        require(end.isAfter(start) && end.epochSecond - start.epochSecond <= 31L * 86_400L) {
            "The calendar event must have a positive duration of at most 31 days"
        }
        val calendarId = writableCalendarId() ?: error("No writable device calendar is available")
        val values = ContentValues().apply {
            put(CalendarContract.Events.CALENDAR_ID, calendarId)
            put(CalendarContract.Events.TITLE, title)
            put(CalendarContract.Events.DTSTART, start.toEpochMilli())
            put(CalendarContract.Events.DTEND, end.toEpochMilli())
            put(CalendarContract.Events.EVENT_TIMEZONE, ZoneId.systemDefault().id)
            request.arguments.text("notes")?.let { put(CalendarContract.Events.DESCRIPTION, it) }
        }
        val uri = context.contentResolver.insert(CalendarContract.Events.CONTENT_URI, values)
            ?: error("The calendar provider rejected the new event")
        return AndroidAgentHostCallResult(
            callId = request.callId,
            output = buildJsonObject {
                put("created", true)
                put("id", ContentUris.parseId(uri).toString())
            },
            isError = false
        )
    }

    private fun writableCalendarId(): Long? {
        val projection = arrayOf(CalendarContract.Calendars._ID)
        val selection = CalendarContract.Calendars.CALENDAR_ACCESS_LEVEL + ">=?"
        val args = arrayOf(CalendarContract.Calendars.CAL_ACCESS_CONTRIBUTOR.toString())
        return context.contentResolver.query(
            CalendarContract.Calendars.CONTENT_URI,
            projection,
            selection,
            args,
            CalendarContract.Calendars.IS_PRIMARY + " DESC"
        )?.use { cursor -> if (cursor.moveToFirst()) cursor.getLong(0) else null }
    }

    private fun AndroidAgentHostCallRequest.scopedUri(): Uri {
        val raw = grant?.text("uri") ?: error("The host call has no scoped URI grant")
        val uri = Uri.parse(raw)
        require(uri.scheme == "content") { "Only Android content URI grants are accepted" }
        return uri
    }

    private fun JsonObject.text(key: String): String? =
        this[key]?.jsonPrimitive?.contentOrNull

    private fun JsonObject.instant(key: String): Instant? =
        text(key)?.let { raw -> runCatching { Instant.parse(raw) }.getOrElse { error("$key must be ISO 8601") } }


    private fun readLimited(input: java.io.InputStream, limit: Int): ByteArray {
        val output = ByteArrayOutputStream(minOf(limit, 32 * 1024))
        val buffer = ByteArray(16 * 1024)
        var total = 0
        while (true) {
            val count = input.read(buffer)
            if (count < 0) break
            total += count
            require(total <= limit) { "Host output exceeds the mobile size limit" }
            output.write(buffer, 0, count)
        }
        return output.toByteArray()
    }
}

internal fun androidAgentFileArtifacts(uri: String, operation: String): JsonArray =
    buildJsonArray {
        val name = uri.substringBefore('?').substringAfterLast('/').ifBlank { "File" }
        add(
            buildJsonObject {
                put("uri", uri)
                put("name", name)
                put("operation", operation)
            }
        )
    }
