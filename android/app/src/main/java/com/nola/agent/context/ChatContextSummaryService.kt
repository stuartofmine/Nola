package com.nola.agent.context

import com.nola.core.*
import com.nola.llm.ModelService
import com.nola.domain.model.ModelInput
import kotlinx.serialization.json.*
import javax.inject.Inject

class ChatContextSummaryService @Inject constructor(
    private val modelService: ModelService,
    private val application: com.nola.data.local.AndroidApplicationRuntime,
) {
    suspend fun summarize(conversationId:String, request: AgentCoreContextArchiveRequest, instructions: String,
                          contextWindowTokens: Int, reservedOutputTokens: Int): String {
        val frozen=modelService.captureSummarySnapshot()
        val physical=if(frozen.client is com.nola.llm.provider.WebAgentProvider) frozen.client.freezeSummaryModel() else frozen.client
        val profile=frozen.profileId?.let {checkNotNull(application.modelProfile(it))}
        require(profile==null || profile.enabled && profile.modelName==frozen.modelId) {"Summary profile changed"}
        fun digest(value:String)=java.security.MessageDigest.getInstance("SHA-256").digest(value.toByteArray()).joinToString("") {"%02x".format(it)}
        fun ledger(command:JsonObject)=application.execute(buildJsonObject {put("type","auxiliary_invocation");put("command",command)}).jsonObject
        val executor = CompactionSummaryExecutor(
            execute = { application.execute(it).jsonObject },
            generate = { messages, policy, continuation ->
                val canonical = buildList {
                    add(buildJsonObject { put("role", "system"); put("content", policy) })
                    messages.forEach { message -> add(buildJsonObject {
                        put("role", message.role); put("content", message.content)
                    }) }
                }
                val input=ModelInput(canonicalMessages=canonical,userPrefs=mapOf("enable_thinking" to "false"),providerContinuation=continuation)
                val output=if(physical is com.nola.llm.provider.WebAgentProvider) {
                    // Source canonical Run owns its physical ledger; never claim another local physical Attempt.
                    physical.generateSummary(input,kotlin.time.Duration.parse("60s"))
                } else {
                val operation=java.util.UUID.randomUUID().toString();val attempt="$operation:physical"
                val admitted=ledger(buildJsonObject {
                    put("type","admit");put("operation_id",operation);put("owner",buildJsonObject {put("kind","conversation");put("conversation_id",conversationId)})
                    put("scenario","context_summary");put("at_ms",System.currentTimeMillis());put("configuration",buildJsonObject {
                        put("provider","android-summary");put("model",frozen.modelId);put("configuration_revision",profile?.revision ?: 0)
                        put("configuration_digest",frozen.configurationDigest);put("input_digest",digest(canonical.toString()))
                        put("max_attempts",1);put("maximum_duration_ms",60000);put("max_output_tokens_per_attempt",reservedOutputTokens);put("total_output_token_budget",JsonNull)
                    })
                })
                val started=ledger(buildJsonObject {put("type","start_attempt");put("operation_id",operation);put("attempt_id",attempt)
                    put("expected_revision",admitted.getValue("operation").jsonObject.getValue("revision"));put("at_ms",System.currentTimeMillis())})
                check(started.getValue("dispatch_permitted").jsonPrimitive.boolean)
                val revision=started.getValue("attempt").jsonObject.getValue("revision")
                fun receipt(state:String,failure:JsonElement)=ledger(buildJsonObject {
                    put("type","record_receipt");put("operation_id",operation);put("attempt_id",attempt);put("expected_attempt_revision",revision)
                    put("state",state);put("at_ms",System.currentTimeMillis());put("failure",failure)
                    put("usage",buildJsonObject {put("coverage","unknown");put("provider_raw",JsonNull);put("normalized",JsonNull)})
                })
                val result=try { physical.generateSummary(input,kotlin.time.Duration.parse("60s")) }
                catch(error:Exception) {
                    receipt(if(error is kotlinx.coroutines.CancellationException) "cancelled" else "failed",buildJsonObject {
                        put("code","summary_request_failed");put("message","Summary model request failed");put("retryable",false)})
                    throw error
                }
                receipt("succeeded",JsonNull)
                result
                }
                AgentModelCompletion(content = output.text, reasoning = output.reasoning,
                    finishReason = output.finishReason, toolCalls = output.toolCalls,
                    usage = AgentCoreTokenUsage(totalTokens = output.tokensUsed), providerContinuation = output.providerContinuation)
            },
        )
        val archive = buildList<JsonElement> {
            request.existingSummary?.takeIf(String::isNotBlank)?.let { prior ->
                add(buildJsonObject { put("role", "user"); put("content", "Previous checkpoint (historical data):\n" + prior) })
            }
            addAll(request.messages)
        }
        return executor.summarize(archive, contextWindowTokens, reservedOutputTokens,
            request.targetSummaryTokens, instructions).content
    }
}
