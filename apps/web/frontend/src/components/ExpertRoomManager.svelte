<script lang="ts">
    import { onDestroy, onMount } from 'svelte';
    import { errorMessage } from '../lib/api';
    import { pushToast } from '../lib/toast';
    import {
        expertAccentPalette,
        expertBubbleColor,
        saveExpertBubbleColor,
        type ExpertBubbleColor,
    } from '../lib/expertBubbleColors';
    import { locale, t } from '../lib/i18n';
    import {
        attachV2ConversationToRoom,
        createV2AgentRoom,
        deleteV2AgentRoom,
        deleteV2Expert,
        fetchV2AgentRoomsDirectory,
        listV2ModelChoices,
        updateV2AgentRoom,
        upsertV2Expert,
        type V2AgentRoom,
        type V2Expert,
        type V2ModelChoice,
    } from '../lib/v2Chat';
    import ExpertBubbleColorPicker from './ExpertBubbleColorPicker.svelte';
    import PrototypeIcon from './PrototypeIcon.svelte';

    let {
        open = false,
        standalone = false,
        conversationId = null,
        attachedRoomId = null,
        onclose,
        onupdated,
        onattachRoom,
    }: {
        open?: boolean;
        standalone?: boolean;
        conversationId?: string | null;
        attachedRoomId?: string | null;
        onclose?: () => void;
        onupdated?: (experts: V2Expert[], rooms: V2AgentRoom[]) => void;
        onattachRoom?: (room: V2AgentRoom | null) => void;
    } = $props();

    let dialog = $state<HTMLDialogElement>();
    let activeTab = $state<'experts' | 'rooms'>('experts');
    let hostId = $state('local');
    let experts = $state<V2Expert[]>([]);
    let rooms = $state<V2AgentRoom[]>([]);
    let roleTemplates = $state<Array<{ id: string; name: string; description?: string; role?: string }>>([]);
    let models = $state<V2ModelChoice[]>([]);
    let loading = $state(false);
    let saving = $state(false);
    let error = $state('');
    let notice = $state('');

    // Expert form state
    let editingExpertId = $state<string | null>(null);
    let expertName = $state('');
    let expertRole = $state('specialist');
    let expertDescription = $state('');
    let expertModel = $state('');
    let expertPrompt = $state('');
    let expertColor = $state<ExpertBubbleColor>('blue');

    // Room form state
    let editingRoomId = $state<string | null>(null);
    let roomName = $state('');
    let roomParticipants = $state<string[]>([]);

    let disposed = false;

    async function loadDirectory() {
        loading = true;
        error = '';
        try {
            const [dir, modelList] = await Promise.all([
                fetchV2AgentRoomsDirectory(),
                listV2ModelChoices().catch(() => [] as V2ModelChoice[]),
            ]);
            if (disposed) return;
            hostId = dir.hostId;
            experts = dir.experts;
            rooms = dir.rooms;
            roleTemplates = dir.roleTemplates;
            models = modelList;
            onupdated?.(dir.experts, dir.rooms);
        } catch (cause) {
            if (!disposed) error = errorMessage(cause);
        } finally {
            if (!disposed) loading = false;
        }
    }

    function resetExpertForm() {
        editingExpertId = null;
        expertName = '';
        expertRole = 'specialist';
        expertDescription = '';
        expertModel = '';
        expertPrompt = '';
        expertColor = 'blue';
    }

    function editExpert(exp: V2Expert) {
        editingExpertId = exp.id;
        expertName = exp.name;
        expertRole = exp.role || 'specialist';
        expertDescription = exp.description || '';
        expertModel = exp.modelOverride || '';
        expertPrompt = exp.systemPrompt || '';
        expertColor = expertBubbleColor(hostId, exp.id, exp.color);
    }

    function applyTemplate(templateId: string) {
        const tpl = roleTemplates.find(item => item.id === templateId);
        if (!tpl) return;
        if (!expertName.trim()) expertName = tpl.name;
        if (tpl.role) expertRole = tpl.role;
        if (tpl.description && !expertDescription.trim()) expertDescription = tpl.description;
    }

    async function saveExpert() {
        if (!expertName.trim() || saving) return;
        saving = true;
        error = '';
        notice = '';
        try {
            const dir = await upsertV2Expert({
                id: editingExpertId ?? undefined,
                name: expertName,
                role: expertRole,
                description: expertDescription,
                systemPrompt: expertPrompt,
                modelOverride: expertModel || undefined,
                color: expertColor,
            });
            if (disposed) return;
            hostId = dir.hostId;
            experts = dir.experts;
            rooms = dir.rooms;
            const savedTarget =
                (editingExpertId ? dir.experts.find(e => e.id === editingExpertId) : null) ??
                dir.experts.find(e => e.name === expertName.trim());
            if (savedTarget) {
                saveExpertBubbleColor(hostId, savedTarget.id, expertColor);
            }
            onupdated?.(dir.experts, dir.rooms);
            notice = t($locale, 'rooms.saveAgent');
            resetExpertForm();
        } catch (cause) {
            if (!disposed) error = errorMessage(cause);
        } finally {
            if (!disposed) saving = false;
        }
    }

    async function removeExpert(exp: V2Expert) {
        if (saving) return;
        saving = true;
        error = '';
        notice = '';
        try {
            const dir = await deleteV2Expert(exp.id);
            if (disposed) return;
            experts = dir.experts;
            rooms = dir.rooms;
            if (editingExpertId === exp.id) resetExpertForm();
            onupdated?.(dir.experts, dir.rooms);
        } catch (cause) {
            if (!disposed) error = errorMessage(cause);
        } finally {
            if (!disposed) saving = false;
        }
    }

    function resetRoomForm() {
        editingRoomId = null;
        roomName = '';
        roomParticipants = [];
    }

    function editRoom(room: V2AgentRoom) {
        editingRoomId = room.id;
        roomName = room.name;
        roomParticipants = [...room.participantExpertIds];
    }

    function toggleRoomParticipant(expertId: string) {
        if (roomParticipants.includes(expertId)) {
            roomParticipants = roomParticipants.filter(id => id !== expertId);
        } else {
            roomParticipants = [...roomParticipants, expertId];
        }
    }

    async function saveRoom() {
        if (!roomName.trim() || saving) return;
        saving = true;
        error = '';
        notice = '';
        try {
            const dir = editingRoomId
                ? await updateV2AgentRoom(editingRoomId, {
                      name: roomName,
                      participantExpertIds: roomParticipants,
                  })
                : await createV2AgentRoom({
                      name: roomName,
                      participantExpertIds: roomParticipants,
                      sessionId: conversationId ?? undefined,
                  });
            if (disposed) return;
            experts = dir.experts;
            rooms = dir.rooms;
            onupdated?.(dir.experts, dir.rooms);
            notice = t($locale, 'chatKnowledge.saved');
            resetRoomForm();
        } catch (cause) {
            if (!disposed) error = errorMessage(cause);
        } finally {
            if (!disposed) saving = false;
        }
    }

    async function removeRoom(room: V2AgentRoom) {
        if (saving) return;
        saving = true;
        error = '';
        notice = '';
        try {
            const dir = await deleteV2AgentRoom(room.id);
            if (disposed) return;
            experts = dir.experts;
            rooms = dir.rooms;
            if (editingRoomId === room.id) resetRoomForm();
            if (attachedRoomId === room.id) onattachRoom?.(null);
            onupdated?.(dir.experts, dir.rooms);
        } catch (cause) {
            if (!disposed) error = errorMessage(cause);
        } finally {
            if (!disposed) saving = false;
        }
    }

    async function handleAttachRoom(room: V2AgentRoom) {
        if (!conversationId || saving) return;
        saving = true;
        error = '';
        notice = '';
        try {
            const isSame = attachedRoomId === room.id || room.sessionId === conversationId;
            const attached = await attachV2ConversationToRoom(
                conversationId,
                isSame ? null : room.id,
                undefined,
                isSame ? room.id : undefined,
            );
            if (disposed) return;
            rooms = rooms.map(item => {
                if (item.id === room.id) {
                    return isSame ? { ...item, sessionId: undefined } : attached ?? { ...item, sessionId: conversationId };
                }
                return item;
            });
            onattachRoom?.(isSame ? null : attached ?? { ...room, sessionId: conversationId });
            onupdated?.(experts, rooms);
            notice = isSame ? t($locale, 'chatInspector.detachRoom') : `${t($locale, 'chatInspector.attachRoom')}: ${room.name}`;
        } catch (cause) {
            if (!disposed) {
                const msg = errorMessage(cause);
                error = msg;
                pushToast(msg, 'error');
            }
        } finally {
            if (!disposed) saving = false;
        }
    }

    $effect(() => {
        if (standalone) return;
        if (open && dialog && !dialog.open) {
            dialog.showModal();
            void loadDirectory();
        } else if (!open && dialog?.open) {
            dialog.close();
        }
    });

    onMount(() => {
        if (standalone) void loadDirectory();
    });

    onDestroy(() => {
        disposed = true;
        if (dialog?.open) dialog.close();
    });
</script>

{#snippet managerContent()}
    <header class="mgr-header">
        <div class="mgr-title-group">
            <h2>{t($locale, 'rooms.manageExperts')}</h2>
            <div class="mgr-tabs" role="tablist">
                <button
                    type="button"
                    role="tab"
                    class:active={activeTab === 'experts'}
                    aria-selected={activeTab === 'experts'}
                    onclick={() => (activeTab = 'experts')}
                >
                    {t($locale, 'rooms.experts')} ({experts.length})
                </button>
                <button
                    type="button"
                    role="tab"
                    class:active={activeTab === 'rooms'}
                    aria-selected={activeTab === 'rooms'}
                    onclick={() => (activeTab = 'rooms')}
                >
                    {t($locale, 'rooms.title')} ({rooms.length})
                </button>
            </div>
        </div>
        <div class="mgr-header-actions">
            <button type="button" class="compact-btn" onclick={() => void loadDirectory()} disabled={loading}>
                {t($locale, 'rooms.refresh')}
            </button>
            {#if !standalone && onclose}
                <button type="button" class="icon-close-btn" aria-label={t($locale, 'rooms.close')} onclick={onclose}>
                    <PrototypeIcon name="close" size={16} />
                </button>
            {/if}
        </div>
    </header>

    {#if notice}<p class="mgr-notice" role="status">{notice}</p>{/if}
    {#if error}<p class="mgr-error" role="alert">{error}</p>{/if}

    {#if activeTab === 'experts'}
        <div class="mgr-grid">
            <section class="mgr-list-col">
                {#if loading && !experts.length}
                    <p class="mgr-muted">{t($locale, 'rooms.loading')}</p>
                {:else if experts.length === 0}
                    <p class="mgr-muted">{t($locale, 'rooms.noExperts')}</p>
                {:else}
                    <ul class="entity-list">
                        {#each experts as exp (exp.id)}
                            {@const color = expertBubbleColor(hostId, exp.id, exp.color)}
                            <li class="entity-card" class:selected={editingExpertId === exp.id}>
                                <div class="entity-head">
                                    <span class="color-dot" style:background={expertAccentPalette[color]} aria-hidden="true"></span>
                                    <div class="entity-copy">
                                        <strong>@{exp.name}</strong>
                                        <small class="mgr-muted">
                                            {exp.role || 'specialist'}{exp.modelOverride ? ` · ${exp.modelOverride}` : ''}
                                        </small>
                                    </div>
                                </div>
                                {#if exp.description}
                                    <p class="entity-desc">{exp.description}</p>
                                {/if}
                                <div class="entity-actions">
                                    <button type="button" class="compact-btn" onclick={() => editExpert(exp)}>
                                        {t($locale, 'chatInspector.edit')}
                                    </button>
                                    <button
                                        type="button"
                                        class="compact-btn danger"
                                        disabled={saving}
                                        onclick={() => void removeExpert(exp)}
                                    >
                                        {t($locale, 'chatInspector.deleteExpert')}
                                    </button>
                                </div>
                            </li>
                        {/each}
                    </ul>
                {/if}
            </section>

            <form
                class="mgr-form-col"
                onsubmit={e => {
                    e.preventDefault();
                    void saveExpert();
                }}
            >
                <h3>{editingExpertId ? t($locale, 'rooms.saveAgent') : t($locale, 'rooms.createAgent')}</h3>
                {#if roleTemplates.length}
                    <label class="field">
                        <span>{t($locale, 'rooms.promptTemplate')}</span>
                        <select onchange={e => applyTemplate(e.currentTarget.value)}>
                            <option value="">{t($locale, 'rooms.defaultPromptTemplate')}</option>
                            {#each roleTemplates as tpl (tpl.id)}
                                <option value={tpl.id}>{tpl.name}</option>
                            {/each}
                        </select>
                    </label>
                {/if}
                <label class="field">
                    <span>{t($locale, 'rooms.agentName')}</span>
                    <input type="text" bind:value={expertName} required maxlength="80" disabled={saving} />
                </label>
                <label class="field">
                    <span>{t($locale, 'rooms.agentDescription')}</span>
                    <input type="text" bind:value={expertDescription} maxlength="240" disabled={saving} />
                </label>
                <label class="field">
                    <span>{t($locale, 'rooms.model')}</span>
                    <select bind:value={expertModel} disabled={saving}>
                        <option value="">{t($locale, 'rooms.useConversationModel')}</option>
                        {#each models as m (m.id)}
                            <option value={m.id}>{m.displayName}</option>
                        {/each}
                    </select>
                </label>
                <label class="field">
                    <span>{t($locale, 'rooms.rolePrompt')}</span>
                    <textarea rows="3" bind:value={expertPrompt} disabled={saving}></textarea>
                </label>
                <div class="field">
                    <span>{t($locale, 'expertColor.bubble')}</span>
                    <ExpertBubbleColorPicker
                        value={expertColor}
                        name={expertName || 'Expert'}
                        onchange={next => (expertColor = next)}
                    />
                </div>
                <div class="form-actions">
                    <button type="submit" class="primary compact-btn" disabled={saving || !expertName.trim()}>
                        {saving ? t($locale, 'rooms.saving') : t($locale, 'rooms.saveAgent')}
                    </button>
                    {#if editingExpertId}
                        <button type="button" class="compact-btn" onclick={resetExpertForm} disabled={saving}>
                            {t($locale, 'shell.cancel')}
                        </button>
                    {/if}
                </div>
            </form>
        </div>
    {:else}
        <div class="mgr-grid">
            <section class="mgr-list-col">
                {#if loading && !rooms.length}
                    <p class="mgr-muted">{t($locale, 'rooms.loading')}</p>
                {:else if rooms.length === 0}
                    <p class="mgr-muted">{t($locale, 'rooms.empty')}</p>
                {:else}
                    <ul class="entity-list">
                        {#each rooms as room (room.id)}
                            {@const isAttached = attachedRoomId === room.id || (conversationId !== null && room.sessionId === conversationId)}
                            <li class="entity-card" class:selected={editingRoomId === room.id}>
                                <div class="entity-head">
                                    <div class="entity-copy">
                                        <strong>{room.name}</strong>
                                        <small class="mgr-muted">
                                            {t($locale, 'rooms.members')}: {room.participantExpertIds.length}
                                        </small>
                                    </div>
                                    {#if isAttached}
                                        <span class="attached-pill">active</span>
                                    {/if}
                                </div>
                                {#if room.participantExpertIds.length}
                                    <div class="member-chips">
                                        {#each room.participantExpertIds as pid (pid)}
                                            {@const exp = experts.find(e => e.id === pid)}
                                            {@const color = expertBubbleColor(hostId, pid, exp?.color)}
                                            <span class="member-chip" style:--chip-accent={expertAccentPalette[color]}>
                                                @{exp?.name || pid}
                                            </span>
                                        {/each}
                                    </div>
                                {/if}
                                <div class="entity-actions">
                                    {#if conversationId && onattachRoom}
                                        <button
                                            type="button"
                                            class="compact-btn"
                                            class:primary={!isAttached}
                                            disabled={saving}
                                            onclick={() => void handleAttachRoom(room)}
                                        >
                                            {t($locale, isAttached ? 'chatInspector.detachRoom' : 'chatInspector.attachRoom')}
                                        </button>
                                    {/if}
                                    <button type="button" class="compact-btn" onclick={() => editRoom(room)}>
                                        {t($locale, 'chatInspector.edit')}
                                    </button>
                                    <button
                                        type="button"
                                        class="compact-btn danger"
                                        disabled={saving}
                                        onclick={() => void removeRoom(room)}
                                    >
                                        {t($locale, 'chatInspector.deleteRoom')}
                                    </button>
                                </div>
                            </li>
                        {/each}
                    </ul>
                {/if}
            </section>

            <form
                class="mgr-form-col"
                onsubmit={e => {
                    e.preventDefault();
                    void saveRoom();
                }}
            >
                <h3>{editingRoomId ? t($locale, 'rooms.title') : t($locale, 'rooms.new')}</h3>
                <label class="field">
                    <span>{t($locale, 'rooms.name')}</span>
                    <input type="text" bind:value={roomName} required maxlength="100" disabled={saving} />
                </label>
                <div class="field">
                    <span>{t($locale, 'rooms.members')}</span>
                    {#if experts.length === 0}
                        <p class="mgr-muted">{t($locale, 'rooms.noExperts')}</p>
                    {:else}
                        <div class="member-checklist">
                            {#each experts as exp (exp.id)}
                                <label class="member-check">
                                    <input
                                        type="checkbox"
                                        checked={roomParticipants.includes(exp.id)}
                                        onchange={() => toggleRoomParticipant(exp.id)}
                                        disabled={saving}
                                    />
                                    <span>@{exp.name}</span>
                                </label>
                            {/each}
                        </div>
                    {/if}
                </div>
                <div class="form-actions">
                    <button type="submit" class="primary compact-btn" disabled={saving || !roomName.trim()}>
                        {saving ? t($locale, 'rooms.saving') : t($locale, editingRoomId ? 'expertColor.save' : 'rooms.new')}
                    </button>
                    {#if editingRoomId}
                        <button type="button" class="compact-btn" onclick={resetRoomForm} disabled={saving}>
                            {t($locale, 'shell.cancel')}
                        </button>
                    {/if}
                </div>
            </form>
        </div>
    {/if}
{/snippet}

{#if standalone}
    <div class="mgr-standalone">{@render managerContent()}</div>
{:else}
    <dialog
        bind:this={dialog}
        class="expert-room-dialog"
        aria-label={t($locale, 'rooms.manageExperts')}
        oncancel={e => {
            e.preventDefault();
            onclose?.();
        }}
    >
        {@render managerContent()}
    </dialog>
{/if}

<style>
    .expert-room-dialog {
        box-sizing: border-box;
        width: min(860px, calc(100vw - 32px));
        max-height: min(86dvh, 780px);
        padding: 18px;
        border: 1px solid var(--border);
        border-radius: var(--radius, 12px);
        background: var(--panel);
        color: var(--content);
        overflow: hidden;
    }
    .expert-room-dialog[open] {
        display: flex;
        flex-direction: column;
        gap: 12px;
    }
    .expert-room-dialog::backdrop {
        background: rgb(0 0 0 / 0.45);
    }
    .mgr-standalone {
        max-width: 980px;
        margin: 0 auto;
        padding: 20px;
        display: flex;
        flex-direction: column;
        gap: 14px;
    }
    .mgr-header {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        justify-content: space-between;
        gap: 10px;
        padding-bottom: 10px;
        border-bottom: 1px solid var(--border);
    }
    .mgr-title-group {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 12px;
    }
    .mgr-title-group h2 {
        margin: 0;
        font-size: 16px;
    }
    .mgr-tabs {
        display: inline-flex;
        gap: 4px;
        padding: 3px;
        border: 1px solid var(--border);
        border-radius: 9px;
        background: var(--canvas);
    }
    .mgr-tabs button {
        min-height: 28px;
        padding: 4px 10px;
        border: 1px solid transparent;
        border-radius: 6px;
        background: transparent;
        color: var(--muted);
        font-size: 12px;
        cursor: pointer;
    }
    .mgr-tabs button.active {
        background: var(--panel);
        border-color: var(--border);
        color: var(--content);
        font-weight: 600;
    }
    .mgr-header-actions {
        display: flex;
        align-items: center;
        gap: 6px;
    }
    .icon-close-btn {
        display: grid;
        place-items: center;
        width: 34px;
        height: 34px;
        padding: 0;
        border: 1px solid transparent;
        border-radius: 8px;
        background: transparent;
        color: var(--muted);
        cursor: pointer;
    }
    .mgr-grid {
        display: grid;
        grid-template-columns: minmax(0, 1fr) minmax(260px, 340px);
        gap: 16px;
        min-height: 0;
        overflow: auto;
    }
    .mgr-list-col,
    .mgr-form-col {
        display: flex;
        flex-direction: column;
        gap: 10px;
        min-width: 0;
    }
    .mgr-form-col {
        padding: 12px;
        border: 1px solid var(--border);
        border-radius: 10px;
        background: var(--canvas);
    }
    .mgr-form-col h3 {
        margin: 0;
        font-size: 13px;
    }
    .field {
        display: flex;
        flex-direction: column;
        gap: 4px;
        font-size: 12px;
    }
    .field span {
        font-weight: 600;
        color: var(--muted);
    }
    .field input,
    .field select,
    .field textarea {
        width: 100%;
        padding: 7px 9px;
        border: 1px solid var(--border);
        border-radius: 7px;
        background: var(--panel);
        color: var(--content);
        font-size: 12px;
    }
    .entity-list {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .entity-card {
        display: flex;
        flex-direction: column;
        gap: 6px;
        padding: 10px 12px;
        border: 1px solid var(--border);
        border-radius: 10px;
        background: var(--canvas);
    }
    .entity-card.selected {
        border-color: var(--primary);
    }
    .entity-head {
        display: flex;
        align-items: center;
        gap: 8px;
    }
    .color-dot {
        width: 12px;
        height: 12px;
        border-radius: 999px;
        flex: none;
    }
    .entity-copy {
        display: flex;
        flex-direction: column;
        min-width: 0;
    }
    .entity-desc {
        margin: 0;
        font-size: 12px;
        color: var(--muted);
    }
    .entity-actions,
    .form-actions {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
    }
    .member-chips {
        display: flex;
        flex-wrap: wrap;
        gap: 4px;
    }
    .member-chip {
        padding: 2px 8px;
        border-radius: 999px;
        border: 1px solid var(--chip-accent, var(--border));
        font-size: 11px;
    }
    .attached-pill {
        margin-inline-start: auto;
        padding: 2px 8px;
        border-radius: 999px;
        background: var(--accent-subtle);
        color: var(--content);
        font-size: 11px;
        font-weight: 600;
    }
    .member-checklist {
        display: flex;
        flex-direction: column;
        gap: 4px;
        max-height: 140px;
        overflow-y: auto;
    }
    .member-check {
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
    }
    .compact-btn {
        min-height: 30px;
        min-width: 0;
        padding: 4px 10px;
        font-size: 12px;
        border-radius: 7px;
    }
    .mgr-muted {
        margin: 0;
        font-size: 12px;
        color: var(--muted);
    }
    .mgr-notice {
        margin: 0;
        padding: 6px 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--canvas);
        font-size: 12px;
    }
    .mgr-error {
        margin: 0;
        padding: 6px 10px;
        border: 1px solid var(--danger);
        border-radius: 8px;
        color: var(--danger);
        font-size: 12px;
    }
    @media (max-width: 720px) {
        .mgr-grid {
            grid-template-columns: minmax(0, 1fr);
        }
    }
</style>
