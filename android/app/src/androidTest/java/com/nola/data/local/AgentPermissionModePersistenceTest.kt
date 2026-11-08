package com.nola.data.local

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.core.AndroidAgentPermissionMode
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AgentPermissionModePersistenceTest {
    @Test
    fun cautiousIsDefaultAndSelectionSurvivesStoreRecreation() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val secureStore = SecureTokenStore(context)
        val first = PreferenceDataStore(context, secureStore)
        first.clearAll()
        try {
            assertEquals(AndroidAgentPermissionMode.Cautious, first.agentPermissionMode.first())
            first.setAgentPermissionMode(AndroidAgentPermissionMode.AllowAll)

            val reopened = PreferenceDataStore(context, secureStore)
            assertEquals(AndroidAgentPermissionMode.AllowAll, reopened.agentPermissionMode.first())
        } finally {
            first.clearAll()
        }
    }
}
