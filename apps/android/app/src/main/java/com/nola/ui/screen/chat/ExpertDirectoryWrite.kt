package com.nola.ui.screen.chat

import com.nola.data.repository.PersistentAgent

/** An existing ID alone is never proof that this editor's attempted write committed. */
internal fun confirmsExpertDefinition(committed: PersistentAgent, attempted: PersistentAgent): Boolean =
    committed.id == attempted.id && committed.revision > attempted.revision && committed.name == attempted.name &&
        committed.description == attempted.description && committed.defaults == attempted.defaults && committed.availability == attempted.availability
