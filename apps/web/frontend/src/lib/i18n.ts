import { expertColorMessages } from './locales/expertColors.ts';
import { get, writable } from 'svelte/store';
import { shellMessages } from './locales/shell.ts';
import { agentRoomMessages } from './locales/agentRooms.ts';
import { modelConnectionMessages } from './locales/modelConnections.ts';
import { sourceAccountMessages } from './locales/sourceAccounts.ts';
import { modelMessages } from './locales/models.ts';
import { credentialMessages } from './locales/credentials.ts';
import { remoteModelMessages } from './locales/remoteModels.ts';
import { activityMessages } from './locales/activity.ts';
import { capabilityMessages } from './locales/capabilities.ts';
import { modelValidationMessages } from './locales/modelValidation.ts';
import { dataSourceMessages } from './locales/dataSources.ts';
import { settingsMessages } from './locales/settings.ts';
import { messagingSettingsMessages } from './locales/messagingSettings.ts';
import { agentAccessMessages } from './locales/agentAccessSettings.ts';
import { automationMessages } from './locales/automations.ts';
import { chatConversationMessages } from './locales/chatConversations.ts';
import { chatPermissionMessages } from './locales/chatPermissions.ts';
import { modelDisplayMessages } from './locales/modelDisplay.ts';
import { chatPageMessages } from './locales/chatPage.ts';
import { chatInspectorMessages } from './locales/chatInspector.ts';
import { chatSessionMessages } from './locales/chatSessionSettings.ts';
import { chatResourceMessages } from './locales/chatResources.ts';
import { chatOverviewMessages } from './locales/chatOverview.ts';
import { chatTextAttachmentMessages } from './locales/chatTextAttachments.ts';
import { chatActivityMessages } from './locales/chatActivity.ts';
import { chatPermissionModeMessages } from './locales/chatPermissionMode.ts';
import { chatDataSourceMessages } from './locales/chatDataSources.ts';
import { chatToolsMessages } from './locales/chatTools.ts';
import { chatBrowserMessages } from './locales/chatBrowser.ts';
import { modelTestMessages } from './locales/modelTest.ts';
import { localModelCenterMessages } from './locales/localModelCenter.ts';
import { executionScopeMessages } from './locales/executionScope.ts';
import { recoveryMessages } from './locales/recovery.ts';
import { chatKnowledgeMessages } from './locales/chatKnowledge.ts';
import { memoryCenterMessages } from './locales/memoryCenter.ts';

export type Locale = 'zh-Hans' | 'en' | 'de' | 'es' | 'fr' | 'ja' | 'ru';
export const supportedLocales: readonly Locale[] = ['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru'];
const preferenceKey = 'nola.web.locale';
const preferenceCookie = 'nola_ui_locale';
export const locale = writable<Locale>('en');
export const localePreference = writable<Locale | 'system'>('system');
export const messageCatalogs: readonly Record<string, Record<string, string>>[] = [
    expertColorMessages,
    chatInspectorMessages,
    agentRoomMessages,
    modelConnectionMessages,
    sourceAccountMessages,
    chatKnowledgeMessages,
    memoryCenterMessages,
    shellMessages, modelMessages, credentialMessages, remoteModelMessages, activityMessages, capabilityMessages, modelValidationMessages, dataSourceMessages,
    settingsMessages, messagingSettingsMessages, agentAccessMessages,
    automationMessages,
    chatConversationMessages,
    chatPermissionMessages,
    modelDisplayMessages,
    chatPageMessages,
    chatSessionMessages,
    chatResourceMessages,
    chatOverviewMessages,
    chatTextAttachmentMessages,
    chatActivityMessages,
    chatPermissionModeMessages,
    chatDataSourceMessages,
    chatToolsMessages,
    chatBrowserMessages,
    modelTestMessages,
    localModelCenterMessages,
    recoveryMessages, executionScopeMessages,
];
const messages = Object.fromEntries(supportedLocales.map(language => [language,
    Object.assign({}, ...messageCatalogs.map(catalog => catalog[language] ?? {})) as Record<string, string>,
])) as Record<Locale, Record<string, string>>;

export function normalizeLocale(value: string): Locale | undefined {
    const base = value.trim().replaceAll('_', '-').toLowerCase().split('-')[0];
    if (base === 'zh') return 'zh-Hans';
    return supportedLocales.find(candidate => candidate === base);
}

export function resolveLocale(preference: string | null, languages: readonly string[]): Locale {
    const explicit = preference && preference !== 'system' ? normalizeLocale(preference) : undefined;
    if (explicit) return explicit;
    for (const language of languages) {
        const match = normalizeLocale(language);
        if (match) return match;
    }
    return 'en';
}

export function t(language: Locale, key: string, params: Record<string, string | number> = {}): string {
    const template = messages[language]?.[key] ?? messages.en[key] ?? key;
    // Values remain text. Never recursively interpolate or turn messages into HTML.
    return template.replace(/\{([A-Za-z][A-Za-z0-9_]*)\}/g, (token, name: string) =>
        Object.hasOwn(params, name) ? String(params[name]) : token);
}

function applyLocale(preference: Locale | 'system') {
    const language = resolveLocale(preference, typeof navigator === 'undefined' ? [] : navigator.languages);
    localePreference.set(preference);
    locale.set(language);
    if (typeof document !== 'undefined') {
        document.documentElement.lang = language;
        // This non-sensitive preference also reaches still-server-rendered pages.
        // Never copy authentication cookies or arbitrary locale input here.
        const secure = typeof location !== 'undefined' && location.protocol === 'https:' ? '; Secure' : '';
        try {
            document.cookie = `${preferenceCookie}=${preference === 'system' ? '' : preference}; Path=/; SameSite=Lax; Max-Age=${preference === 'system' ? 0 : 31536000}${secure}`;
        } catch { /* Cookies can be disabled without preventing a UI choice. */ }
    }
}

export function localeFromCookie(cookie: string): Locale | undefined {
    const matches = cookie.split(';').map(item => item.trim()).filter(item => item.startsWith(`${preferenceCookie}=`));
    if (matches.length !== 1) return;
    const value = matches[0].slice(preferenceCookie.length + 1);
    return supportedLocales.find(language => language === value);
}

export function setLocale(preference: Locale | 'system') {
    if (preference !== 'system' && !supportedLocales.includes(preference)) return;
    try { localStorage.setItem(preferenceKey, preference); } catch { /* Privacy modes can disable storage. */ }
    applyLocale(preference);
}

export function initializeLocale() {
    let preference: string | null = null;
    try { preference = localStorage.getItem(preferenceKey); } catch { /* Browser-language fallback. */ }
    if (preference === null && typeof document !== 'undefined') {
        try { preference = localeFromCookie(document.cookie || '') ?? null; } catch { /* Cookies can be disabled too. */ }
    }
    applyLocale(preference ? normalizeLocale(preference) ?? 'system' : 'system');
}

/** Follow browser changes only in system mode; sync explicit choices across tabs. */
export function observeLocaleChanges(target: Window = window): () => void {
    const languageChanged = () => { if (get(localePreference) === 'system') applyLocale('system'); };
    const preferenceChanged = (event: StorageEvent) => {
        if (event.key === preferenceKey || event.key === null) initializeLocale();
    };
    target.addEventListener('languagechange', languageChanged);
    target.addEventListener('storage', preferenceChanged);
    return () => {
        target.removeEventListener('languagechange', languageChanged);
        target.removeEventListener('storage', preferenceChanged);
    };
}
