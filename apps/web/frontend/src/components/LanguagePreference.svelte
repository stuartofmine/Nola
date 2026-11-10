<script lang="ts">
    import {locale, localePreference, supportedLocales, setLocale, t, type Locale} from '../lib/i18n';
    import {languageNames} from '../lib/locales/shell';
</script>
<label class="language-preference">{t($locale, 'shell.language')}
    <select value={$localePreference} onchange={event => setLocale(event.currentTarget.value as Locale | 'system')}>
        <option value="system">{t($locale, 'shell.language.system')}</option>
        {#each supportedLocales as language}<option value={language} lang={language}>{languageNames[language]}</option>{/each}
    </select>
</label>
<style>.language-preference { display: grid; gap: 8px; font-size: .875rem; } select { width: 100%; min-width: 0; min-height: 44px; }</style>
