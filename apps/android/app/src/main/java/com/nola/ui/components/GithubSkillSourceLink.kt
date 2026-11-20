// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.components

import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import com.nola.domain.model.UploadedContent
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

@Composable
internal fun GithubSkillSourceLink(skill: UploadedContent?) {
    val reference = skill?.canonicalSkillRecord?.get("instructions_ref")?.jsonPrimitive?.contentOrNull ?: return
    val prefix = "nola://github-skills/"
    if (!reference.startsWith(prefix)) return
    val path = reference.removePrefix(prefix)
    val uri = LocalUriHandler.current
    TextButton(onClick = { uri.openUri("https://github.com/$path") }, modifier = Modifier.testTag("skill.editor.github")) {
        Text("GitHub · " + path.substringBefore("/tree/"))
    }
}
