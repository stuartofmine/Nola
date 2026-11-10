<script lang="ts">
    import { useProjection, inheritedPresentationOnly } from './context';
    import Slot from './Slot.svelte';
    import type { Binding, SlotContent } from '../catalog/types';
    import Text from '../catalog/basic/Text.svelte';
    import Row from '../catalog/basic/Row.svelte';
    import Column from '../catalog/basic/Column.svelte';
    import List from '../catalog/basic/List.svelte';
    import Card from '../catalog/basic/Card.svelte';
    import Tabs from '../catalog/basic/Tabs.svelte';
    import Modal from '../catalog/basic/Modal.svelte';
    import Divider from '../catalog/basic/Divider.svelte';
    import Button from '../catalog/basic/Button.svelte';
    import TextField from '../catalog/basic/TextField.svelte';
    import CheckBox from '../catalog/basic/CheckBox.svelte';
    import ChoicePicker from '../catalog/basic/ChoicePicker.svelte';
    import Slider from '../catalog/basic/Slider.svelte';
    import DateTimeInput from '../catalog/basic/DateTimeInput.svelte';
    import Icon from '../catalog/basic/Icon.svelte';
    let { id }: { id: string } = $props();
    // Native DOM identity is per mount; Core instance IDs may intentionally
    // recur in a read-only history beside the live Surface.
    const domId = $props.id();
    const context = useProjection();
    const presentationOnly = inheritedPresentationOnly();
    const node = $derived(context.snapshot().nodes[id]);
    const components = { Text, Row, Column, List, Card, Tabs, Modal, Divider, Button,
        TextField, CheckBox, ChoicePicker, Slider, DateTimeInput, Icon };
    const Component = $derived(node ? components[node.component as keyof typeof components] : undefined);
    const slots = $derived.by(() => {
        const result: Record<string, SlotContent> = {};
        if (!node) return result;
        for (const [name, ids] of Object.entries(node.slots)) {
            result[name] = { kind: 'nodes', nodes: ids.map(id => ({ id, key: id })) };
        }
        if (Array.isArray(node.props.tab_titles)) {
            result.tabs = { kind: 'tabs', tabs: node.props.tab_titles.map((title, index) => ({
                title: String(title ?? ''), nodes: (node.slots[`tab:${index}`] ?? []).map(id => ({ id, key: id }))
            })) };
        }
        return result;
    });
    const bindings = $derived.by(() => {
        const result: Record<string, Binding> = {};
        if (!node) return result;
        for (const [name, path] of Object.entries(node.bindings)) {
            result[name] = { value: context.value(id, name, node.props[name]), path,
                set: value => context.edit(id, name, value) };
        }
        return result;
    });
    const actions = $derived.by(() => Object.fromEntries(Object.entries(node?.actions ?? {})
        .map(([name, reference]) => [name, () => context.dispatch(reference)])));
    const validation = $derived({ valid: !node?.validation_errors.length, errors: node?.validation_errors ?? [] });
</script>
{#if node && presentationOnly() && (node.component === 'Button' || node.component === 'Modal')}
    <Slot content={slots[node.component === 'Button' ? 'child' : 'trigger']} presentationOnly />
{:else if node && presentationOnly() && ['TextField', 'CheckBox', 'ChoicePicker', 'Slider', 'DateTimeInput'].includes(node.component)}
    <span>{String(node.props.label ?? node.props.ariaLabel ?? '')}</span>
{:else if node && Component}
    <Component {...node.props} {slots} {bindings} {actions} {validation}
        disabled={presentationOnly() || context.disabled() || node.props.disabled === true
            || Object.keys(node.actions).length > 0 && context.actionsDisabled?.() === true
            || ['TextField', 'CheckBox', 'ChoicePicker', 'Slider', 'DateTimeInput'].includes(node.component) && !node.bindings.value} a2ui={{ id: domId }} />
{:else if node}
    <p role="status">{node.component}: unsupported renderer component</p>
{/if}
