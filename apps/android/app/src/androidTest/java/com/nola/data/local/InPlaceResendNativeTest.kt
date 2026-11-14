// SPDX-License-Identifier: Apache-2.0
package com.nola.data.local

import androidx.test.core.app.ApplicationProvider
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.UUID

class InPlaceResendNativeTest {
    @Test fun textWithIndependentMetadataRemainsEditableAcrossClients() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir,"in-place-metadata-${UUID.randomUUID()}").apply { mkdirs() }
        try {
            AndroidApplicationRuntime(context,File(root,"metadata.db")).use { runtime ->
                runtime.createConversation("metadata","Imported text",1,1,null)
                runtime.execute(buildJsonObject {
                    put("type","import_conversation_transcript");put("input",buildJsonObject {
                        put("import_id",UUID.randomUUID().toString());put("source","external_archive")
                        put("conversation_id","metadata");put("expected_message_count",0);put("imported_at_ms",2)
                        put("messages",buildJsonArray { add(buildJsonObject {
                            put("id","user");put("role","user");put("status","completed");put("ordinal",1)
                            put("created_at_ms",2);put("completed_at_ms",2)
                            put("parts",buildJsonArray { add(buildJsonObject {
                                put("id","user:text");put("kind","text");put("ordinal",1);put("state","completed")
                                put("content_text","text with metadata");put("content_json",buildJsonObject { put("nola.canonicalMessageID","user") }.toString())
                                put("created_at_ms",2);put("updated_at_ms",2)
                            }) })
                        }) })
                    })
                })
                val projection = runtime.loadConversationProjectionV1("metadata")
                runtime.prepareInPlaceResend(AndroidPrepareInPlaceResend(UUID.randomUUID().toString(),"metadata","user",
                    checkNotNull(runtime.loadConversation("metadata")).revision,projection.messages.single().revision,"edited",3))
                assertTrue(checkNotNull(runtime.loadConversation("metadata")).messages.isEmpty())
            }
        } finally { root.deleteRecursively() }
    }
    @Test fun sameOwnerRewindAndColdRetryUseRealCoreWithoutReplacingNewContent() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir,"in-place-${UUID.randomUUID()}").apply { mkdirs() }
        val database = File(root,"isolated.db")
        val owner = UUID.randomUUID().toString()
        lateinit var prepared: AndroidPreparedInPlaceResend
        try {
            AndroidApplicationRuntime(context,database).use { runtime ->
                runtime.createConversation(owner,"Keep title",1,1,null)
                for (i in 0..2) runtime.execute(buildJsonObject {
                    put("type","admit_conversation_user_message");put("input",buildJsonObject {
                        put("conversation_id",owner);put("message_id","u$i");put("text","original-$i");put("created_at_ms",i+2)
                    })
                })
                val projection = runtime.loadConversationProjectionV1(owner)
                val revision = checkNotNull(runtime.loadConversation(owner)).revision
                prepared = runtime.prepareInPlaceResend(AndroidPrepareInPlaceResend(UUID.randomUUID().toString(),owner,"u1",
                    revision,projection.messages.single { it.messageId=="u1" }.revision,"replacement",10))
                assertEquals(owner,prepared.input.conversationId)
                assertEquals(listOf("u0"),checkNotNull(runtime.loadConversation(owner)).messages.map { it.id })
            }
            AndroidApplicationRuntime(context,database).use { runtime ->
                runtime.execute(buildJsonObject {
                    put("type","admit_conversation_user_message");put("input",buildJsonObject {
                        put("conversation_id",owner);put("message_id",prepared.userMessageId);put("text","replacement");put("created_at_ms",10)
                    })
                })
                assertEquals(prepared,runtime.prepareInPlaceResend(prepared.input))
                assertEquals(listOf("u0",prepared.userMessageId),checkNotNull(runtime.loadConversation(owner)).messages.map { it.id })
                assertEquals(1,runtime.listConversations().size)
                assertEquals("Keep title",checkNotNull(runtime.loadConversation(owner)).title)
                val saved=runtime.executeQuery(buildJsonObject { put("type","load_prepared_in_place_resend");put("operation_id",prepared.runId) })
                assertEquals(prepared.runId,saved.jsonObject.getValue("run_id").jsonPrimitive.content)
            }
        } finally { root.deleteRecursively() }
    }
}
