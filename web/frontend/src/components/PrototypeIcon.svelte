<script lang="ts">
    import sidebar from '../../../../assets/icons/sidebar.svg?raw';
    import sun from '../../../../assets/icons/sun.svg?raw';
    import chat from '../../../../assets/icons/chat.svg?raw';
    import folder from '../../../../assets/icons/folder.svg?raw';
    import settings from '../../../../assets/icons/settings.svg?raw';
    import models from '../../../../assets/icons/models.svg?raw';
    import clock from '../../../../assets/icons/clock.svg?raw';
    import plus from '../../../../assets/icons/plus.svg?raw';
    import search from '../../../../assets/icons/search.svg?raw';
    import overview from '../../../../assets/icons/overview.svg?raw';
    import review from '../../../../assets/icons/review.svg?raw';
    import artifacts from '../../../../assets/icons/artifacts.svg?raw';
    import progress from '../../../../assets/icons/progress.svg?raw';
    import goal from '../../../../assets/icons/goal.svg?raw';
    import close from '../../../../assets/icons/close.svg?raw';
    import more from '../../../../assets/icons/more.svg?raw';
    import file from '../../../../assets/icons/file.svg?raw';
    import person from '../../../../assets/icons/person.svg?raw';
    import arrow from '../../../../assets/icons/arrow.svg?raw';
    import stop from '../../../../assets/icons/stop.svg?raw';
    import copy from '../../../../assets/icons/copy.svg?raw';
    import edit from '../../../../assets/icons/edit.svg?raw';
    import moon from '../../../../assets/icons/moon.svg?raw';
    import back from '../../../../assets/icons/back.svg?raw';
    import chevron from '../../../../assets/icons/chevron.svg?raw';
    const icons = {sidebar, sun, chat, folder, settings, models, clock, plus, search, overview, review, artifacts, progress, goal, close, more, file, person, arrow, stop, copy, edit, moon, chevron, back};
    let {name, size = 20}: {name: keyof typeof icons; size?: number} = $props();
</script>
<!-- Only repository-owned SVG source assets; labels belong to the containing control. -->
<span class="prototype-icon" style:width={`${size}px`} style:height={`${size}px`} aria-hidden="true">{@html icons[name]}</span>
<style>
    .prototype-icon { display: inline-flex; flex-shrink: 0; }
    .prototype-icon :global(svg) { width: 100%; height: 100%; }
</style>
