<script lang="ts">
    import { locale, t } from '../lib/i18n';
    import { sourceAccountStateKey } from '../lib/locales/sourceAccounts';
    import { onDestroy, onMount } from 'svelte';
    import { api } from '../lib/api';
    type Prompt = {type: 'text' | 'select'; key: string; message: string; placeholder?: string; options?: {label: string; value: string}[]; when?: {key: string; op: string; value: string}};
    type Method = {provider: string; method: number; binding: string; declaration: {type: string; label: string; prompts: Prompt[]}};
    type Owner = {account_id: string; revision: number; display_name: string; origin: string; directory: string};
    type Connected = {account: {revision: number; account: {id: string}}; origin: string; directory: string; methods: Method[]};
    type Login = {request_id: string; provider: string; method: number; state: string; authorization_url: string | null; callback_mode: string | null; instructions: string | null};
    let owners = $state<Owner[]>([]), ownerID = $state(''), connected = $state<Connected | null>(null);
    let origin = $state(''), directory = $state(''), username = $state('opencode'), password = $state('');
    let consent = $state(false), loginConsent = $state(false), callbackConsent = $state(false);
    let selected = $state(''), inputs = $state<Record<string, string>>({}), code = $state('');
    let login = $state<Login | null>(null), busy = $state(false), failed = $state(false);
    let accountID = crypto.randomUUID();
    let alive = true;
    const method = $derived(connected?.methods.find(value => value.binding + ':' + value.provider + ':' + value.method === selected));
    function visible(prompt: Prompt) {return !prompt.when || (prompt.when.op === 'eq' ? inputs[prompt.when.key] === prompt.when.value : inputs[prompt.when.key] !== prompt.when.value);}
    function reference() {if (!connected) throw new Error('owner missing'); return {account_id: connected.account.account.id, expected_revision: connected.account.revision};}
    function selectOwner() {
        connected = null; selected = ''; inputs = {}; code = '';
        consent = false; loginConsent = false; callbackConsent = false; failed = false;
    }
    async function post<T>(action: string, body: unknown) {return api<T>('/api/model-connections/opencode-login/' + action, {method: 'POST', body: JSON.stringify(body)});}
    async function run(work: () => Promise<void>) {if (busy) return; busy = true; failed = false; try {await work();} catch {if (alive) failed = true;} finally {if (alive) busy = false;}}
    async function refreshOwners() {await run(async () => {const result = await post<Owner[]>('owners', {}); if (alive) owners = result;});}
    async function connect() {await run(async () => {
        const result = await post<Connected>('connect', {account_id: accountID, expected_revision: null, display_name: 'OpenCode', origin, directory, username, password, acknowledge_local_service_access: consent});
        if (alive) {
            connected = result; password = ''; selected = ''; inputs = {};
            ownerID = result.account.account.id;
            owners = [...owners.filter(owner => owner.account_id !== ownerID), {
                account_id: ownerID, revision: result.account.revision,
                display_name: 'OpenCode', origin: result.origin, directory: result.directory,
            }];
            accountID = crypto.randomUUID();
            consent = false; loginConsent = false; callbackConsent = false;
        }
    });}
    async function inspect() {await run(async () => {
        const owner = owners.find(value => value.account_id === ownerID); if (!owner) return;
        const result = await post<Connected>('inspect', {account_id: owner.account_id, expected_revision: owner.revision});
        if (alive) {connected = result; selected = ''; inputs = {}; loginConsent = false; callbackConsent = false;}
    });}
    async function authorize() {if (!method || !loginConsent || login) return; const chosen = method;
        await run(async () => {
            const request_id = crypto.randomUUID();
            // Keep an uncertain handle even when the HTTP receipt is lost; never auto-repeat POST.
            login = {request_id, provider: chosen.provider, method: chosen.method, state: 'unknown', authorization_url: null, callback_mode: null, instructions: null};
            const values = Object.fromEntries(chosen.declaration.prompts.filter(visible).map(prompt => [prompt.key, inputs[prompt.key] ?? '']));
            const result = await post<Login>('authorize', {...reference(), request_id, provider: chosen.provider, method: chosen.method, method_binding: chosen.binding, inputs: values, acknowledge_provider_login_changes: true});
            if (alive) {login = result; inputs = {}; callbackConsent = false;}
        });
    }
    async function finish(forget: boolean) {if (!login) return; const current = login;
        await run(async () => {
            if (!forget) login = {...current, state: 'unknown', authorization_url: null, instructions: null};
            const result = await post<Login>(forget ? 'forget' : 'callback', {...reference(), request_id: current.request_id, code: forget ? null : code || null, acknowledge_provider_login_changes: !forget && callbackConsent});
            if (alive) {login = forget ? null : result; code = ''; loginConsent = false; callbackConsent = false;}
        });
    }
    onMount(() => {void refreshOwners();});
    // Unmount never calls a source callback or cancellation endpoint.
    onDestroy(() => {alive = false; password = ''; code = ''; inputs = {};});
</script>

<section class="source-login">
    <h4>{t($locale, 'sourceAccounts.openCodeTitle')}</h4>
    <p>{t($locale, 'sourceAccounts.openCodeLimits')}</p>
    <fieldset disabled={busy || login !== null}>
        <label>{t($locale, 'sourceAccounts.savedOwner')} <select bind:value={ownerID} onchange={selectOwner}><option value="">{t($locale, 'sourceAccounts.selectOwner')}</option>{#each owners as owner}<option value={owner.account_id}>{owner.display_name} · {owner.origin} · {owner.directory}</option>{/each}</select></label>
        <button onclick={inspect} disabled={!ownerID}>{t($locale, 'sourceAccounts.inspectOwner')}</button><button onclick={refreshOwners}>{t($locale, 'sourceAccounts.refreshOwners')}</button>
        <details><summary>{t($locale, 'sourceAccounts.connectOwner')}</summary>
            <label>{t($locale, 'sourceAccounts.origin')}<input bind:value={origin} placeholder="http://127.0.0.1:4096" /></label>
            <label>{t($locale, 'sourceAccounts.directory')} <input bind:value={directory} placeholder="/absolute/project" /></label>
            <label>{t($locale, 'sourceAccounts.username')} <input bind:value={username} autocomplete="off" /></label>
            <label>{t($locale, 'sourceAccounts.password')} <input bind:value={password} type="password" autocomplete="new-password" /></label>
            <label><input type="checkbox" bind:checked={consent} /> {t($locale, 'sourceAccounts.openCodeConsent')}</label>
            <button onclick={connect} disabled={!consent || !origin || !directory || !password}>{t($locale, 'sourceAccounts.connectOwner')}</button>
        </details>
    </fieldset>
    {#if connected}
        <p>{connected.origin} · {connected.directory}</p>
        <fieldset disabled={busy || login !== null}>
            <label>{t($locale, 'sourceAccounts.providerMethod')} <select bind:value={selected} onchange={() => {inputs = {}; loginConsent = false;}}><option value="">{t($locale, 'sourceAccounts.selectMethod')}</option>{#each connected.methods.filter(value => value.declaration.type === 'oauth') as entry}<option value={entry.binding + ':' + entry.provider + ':' + entry.method}>{entry.provider} · {entry.declaration.label}</option>{/each}</select></label>
            {#if method}{#each method.declaration.prompts as prompt}{#if visible(prompt)}
                <label>{prompt.message}{#if prompt.type === 'select'}<select value={inputs[prompt.key] ?? ''} onchange={event => inputs = {...inputs, [prompt.key]: event.currentTarget.value}}><option value="">{t($locale, 'sourceAccounts.select')}</option>{#each prompt.options ?? [] as option}<option value={option.value}>{option.label}</option>{/each}</select>{:else}<input type="password" autocomplete="off" placeholder={prompt.placeholder ?? ''} value={inputs[prompt.key] ?? ''} oninput={event => inputs = {...inputs, [prompt.key]: event.currentTarget.value}} />{/if}</label>
            {/if}{/each}{/if}
            <label><input type="checkbox" bind:checked={loginConsent} /> {t($locale, 'sourceAccounts.openCodeLoginConsent')}</label>
            <button onclick={authorize} disabled={!method || !loginConsent}>{t($locale, 'sourceAccounts.startSelected')}</button>
        </fieldset>
    {/if}
    {#if login}
        <p>{t($locale, 'sourceAccounts.providerStatus', {provider: login.provider, state: t($locale, sourceAccountStateKey(login.state))})}</p>
        {#if login.authorization_url}<a href={login.authorization_url} target="_blank" rel="noopener noreferrer">{t($locale, 'sourceAccounts.openAuthorization', {host: new URL(login.authorization_url).host})}</a>{/if}
        {#if login.instructions}<p class="instructions">{login.instructions}</p>{/if}
        {#if login.state === 'pending'}
            {#if login.callback_mode === 'code'}<label>{t($locale, 'sourceAccounts.code')} <input type="password" bind:value={code} autocomplete="off" disabled={busy} /></label>{/if}
            <label><input type="checkbox" bind:checked={callbackConsent} disabled={busy} /> {t($locale, 'sourceAccounts.callbackConsent')}</label>
            <button onclick={() => finish(false)} disabled={busy || !callbackConsent || (login.callback_mode === 'code' && !code)}>{t($locale, 'sourceAccounts.confirmCallback')}</button>
        {/if}
        <button onclick={() => finish(true)} disabled={busy}>{t($locale, 'sourceAccounts.forgetHandle')}</button>
    {/if}
    {#if failed}<p role="alert">{t($locale, 'sourceAccounts.openCodeError')}</p>{/if}
</section>

<style>
    .source-login {border: 1px solid var(--border); border-radius: var(--radius); padding: 16px; margin-top: 16px; min-width: 0; overflow-wrap: anywhere;}
    fieldset {border: 0; padding: 0; min-width: 0;}
    label {display: block; margin: 8px 0;}
    input:not([type='checkbox']), select {max-width: 100%; width: 100%; box-sizing: border-box;}
    button {margin: 4px 8px 4px 0;}
    .instructions {white-space: pre-wrap;}
</style>
