// SPDX-License-Identifier: Apache-2.0
(function (root) {
    'use strict';

    const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
    const LEGACY_SESSION = /^Session [0-9a-f]{8}$/i;

    function isRuntimePlaceholder(title, conversationId) {
        const normalized = String(title || '').trim();
        if (!normalized || ['New conversation', 'New Conversation', '新对话'].includes(normalized)) return true;
        if (UUID.test(normalized) || LEGACY_SESSION.test(normalized)) return true;
        const canonicalId = String(conversationId || '').trim();
        if (!canonicalId) return false;
        return normalized === canonicalId || normalized === `Conversation ${canonicalId.slice(0, 8)}`;
    }

    function displayTitle(title, conversationId, fallback) {
        const normalized = String(title || '').trim();
        return isRuntimePlaceholder(normalized, conversationId) ? fallback : normalized;
    }

    root.NolaConversationTitle = Object.freeze({ displayTitle, isRuntimePlaceholder });
})(typeof window !== 'undefined' ? window : globalThis);
