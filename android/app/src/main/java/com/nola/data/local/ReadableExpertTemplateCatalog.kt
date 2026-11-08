// SPDX-License-Identifier: Apache-2.0
package com.nola.data.local

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.*

@Serializable
data class ReadableExpertTemplateChoice(val id: String, val name: String)
@Serializable
data class ReadableExpertTemplateCatalog(val revision: Long?, val templates: List<ReadableExpertTemplateChoice>)

/** Names only. The authenticated native factory supplies the installation actor;
 * Core owns catalog parsing, real authorization time and ReadContent policy. */
internal fun AndroidApplicationRuntime.readableExpertTemplates(): ReadableExpertTemplateCatalog =
    expertHostFactory.readableTemplates()
