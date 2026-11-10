const preferenceKey = 'nola.web.locale';
const cookieName = 'nola_ui_locale';
const supported = new Set(['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru']);

export function savedLocalePreference(storage, cookie) {
    let stored = null;
    try { stored = storage?.getItem(preferenceKey) ?? null; } catch { /* Storage may be disabled. */ }
    if (stored === 'system' || supported.has(stored)) return stored;
    const matches = String(cookie || '').split(';').map(part => part.trim()).filter(part => part.startsWith(`${cookieName}=`));
    if (matches.length !== 1) return 'system';
    const value = matches[0].slice(cookieName.length + 1);
    return supported.has(value) ? value : 'system';
}

export function writeLocalePreference(value, storage, document, location) {
    if (value !== 'system' && !supported.has(value)) return false;
    const secure = location?.protocol === 'https:' ? '; Secure' : '';
    try {
        document.cookie = `${cookieName}=${value === 'system' ? '' : value}; Path=/; SameSite=Lax; Max-Age=${value === 'system' ? 0 : 31536000}${secure}`;
    } catch { return false; }
    try { storage?.setItem(preferenceKey, value); } catch { /* Cookie still persists the choice. */ }
    return true;
}

export function bindLocaleSelect(select, environment = globalThis) {
    if (!select) return () => {};
    const { document, location, window } = environment;
    let localStorage;
    try { localStorage = environment.localStorage; } catch { /* Storage may be disabled. */ }
    const sync = () => { select.value = savedLocalePreference(localStorage, document.cookie); };
    const changed = () => {
        const draft = document.getElementById('userInput')?.value?.trim();
        if (draft && !window.confirm(select.dataset.draftWarning || 'Changing language will discard your unsent message. Continue?')) {
            sync();
            return;
        }
        if (writeLocalePreference(select.value, localStorage, document, location)) window.location.reload();
        else sync();
    };
    sync();
    select.addEventListener('change', changed);
    window.addEventListener('storage', sync);
    return () => { select.removeEventListener('change', changed); window.removeEventListener('storage', sync); };
}

if (typeof document !== 'undefined') bindLocaleSelect(document.getElementById('chatLanguageSelect'));
