// SPDX-License-Identifier: Apache-2.0
// Preserve mounted forms and unsaved drafts when switching model-center tabs.
(function (root) {
    'use strict';
    function mount(container, document) {
        const tabs = Array.from(container.querySelectorAll('[role="tab"]'));
        function select(index, focus) {
            tabs.forEach((tab, current) => {
                const selected = current === index;
                tab.setAttribute('aria-selected', String(selected));
                tab.tabIndex = selected ? 0 : -1;
                const panel = document.getElementById(tab.getAttribute('aria-controls'));
                if (panel) panel.hidden = !selected;
            });
            if (focus) tabs[index].focus();
        }
        tabs.forEach((tab, index) => {
            tab.addEventListener('click', () => select(index, false));
            tab.addEventListener('keydown', event => {
                let next;
                if (event.key === 'ArrowRight') next = (index + 1) % tabs.length;
                else if (event.key === 'ArrowLeft') next = (index + tabs.length - 1) % tabs.length;
                else if (event.key === 'Home') next = 0;
                else if (event.key === 'End') next = tabs.length - 1;
                else return;
                event.preventDefault();
                select(next, true);
            });
        });
    }
    if (typeof module !== 'undefined') module.exports = { mount };
    if (root.document) {
        root.document.querySelectorAll('.model-center-tabs').forEach(container => mount(container, root.document));
    }
})(typeof window !== 'undefined' ? window : globalThis);
