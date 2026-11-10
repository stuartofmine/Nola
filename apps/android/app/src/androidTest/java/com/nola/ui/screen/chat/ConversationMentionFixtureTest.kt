// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.test.platform.app.InstrumentationRegistry
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.repository.AgentRoomRepository
import android.database.sqlite.SQLiteDatabase
import java.io.File
import kotlinx.serialization.json.*
import org.junit.Assume.assumeTrue
import org.junit.Test

/** Seeds only an explicitly isolated emulator test user; UI acceptance is a separate run. */
class ConversationMentionFixtureTest {
    @Test fun configureExpertsThroughCore() {
        assumeTrue(InstrumentationRegistry.getArguments().getString("mentionFixture") == "true")
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        check(context.noBackupFilesDir.path.contains("/10/")) { "Requires dedicated acceptance user 10" }
        val db = SQLiteDatabase.openDatabase(File(context.noBackupFilesDir, "nola-product.db").path, null, SQLiteDatabase.OPEN_READONLY)
        val defaults = db.use {
            it.rawQuery("SELECT settings_json FROM conversation_settings", null).use { cursor ->
                var selected: JsonObject? = null
                while (cursor.moveToNext()) {
                    val settings = Json.parseToJsonElement(cursor.getString(0)).jsonObject
                    if (settings["modelSelectionID"] != null) selected = JsonObject(settings.filterKeys { key -> key in setOf("modelSelectionID", "modelOverrides") })
                }
                checkNotNull(selected) { "Requires a configured real-model conversation" }
            }
        }
        AndroidApplicationRuntime(context).use { runtime ->
            val repository = AgentRoomRepository(runtime)
            for (name in listOf("Arithmetic QA", "Language QA")) {
                val directory = repository.directory()
                if (directory.agents.none { it.name == name }) repository.createAgent(name, defaults, directory, "Acceptance expert")
            }
            check(repository.directory().agents.count { it.name in setOf("Arithmetic QA", "Language QA") } == 2)
        }
    }
}
