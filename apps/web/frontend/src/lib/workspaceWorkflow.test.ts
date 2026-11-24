// SPDX-License-Identifier: Apache-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import {
    classifyDiffLine,
    fileStatusTone,
    isValidWorktreeBranchName,
    parseUnifiedDiffLines,
} from './workspaceWorkflow.ts';

test('parseUnifiedDiffLines classifies file headers, hunks, additions, deletions, and context', () => {
    const sample = [
        'diff --git a/src/app.ts b/src/app.ts',
        'index 1111111..2222222 100644',
        '--- a/src/app.ts',
        '+++ b/src/app.ts',
        '@@ -1,3 +1,3 @@',
        ' const a = 1;',
        '-const b = 2;',
        '+const b = 3;',
        '',
    ].join('\n');

    const lines = parseUnifiedDiffLines(sample);
    assert.equal(lines.length, 8);
    assert.deepEqual(
        lines.map(line => line.kind),
        ['file', 'file', 'file', 'file', 'hunk', 'context', 'del', 'add'],
    );
    assert.equal(classifyDiffLine('+++ b/foo'), 'file');
    assert.equal(classifyDiffLine('--- /dev/null'), 'file');
    assert.equal(classifyDiffLine('+added'), 'add');
    assert.equal(classifyDiffLine('-removed'), 'del');
    assert.equal(classifyDiffLine('@@ -10,2 +10,4 @@ fn'), 'hunk');
    assert.deepEqual(parseUnifiedDiffLines(''), []);
});

test('fileStatusTone and isValidWorktreeBranchName validate workflow metadata', () => {
    assert.equal(fileStatusTone('M'), 'mod');
    assert.equal(fileStatusTone('A'), 'add');
    assert.equal(fileStatusTone('D'), 'del');
    assert.equal(fileStatusTone('??'), 'untracked');
    assert.equal(fileStatusTone('R'), 'rename');

    assert.equal(isValidWorktreeBranchName('feat/diff-viewer_v1.2'), true);
    assert.equal(isValidWorktreeBranchName(''), false);
    assert.equal(isValidWorktreeBranchName('-flag'), false);
    assert.equal(isValidWorktreeBranchName('feat..bad'), false);
    assert.equal(isValidWorktreeBranchName('bad branch'), false);
    assert.equal(isValidWorktreeBranchName('feat/test.lock'), false);
});

test('WorkspaceDiffPanel renders cleanly via Svelte SSR across all locales', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: WorkspaceDiffPanel } = await server.ssrLoadModule('/src/components/WorkspaceDiffPanel.svelte');
        const { locale, supportedLocales, t } = await server.ssrLoadModule('/src/lib/i18n.ts');
        const { render } = await server.ssrLoadModule('svelte/server');

        for (const language of supportedLocales) {
            locale.set(language);
            const closedHtml: string = render(WorkspaceDiffPanel, { props: { open: false } }).body;
            assert.equal(closedHtml.includes('workflow-drawer'), false);

            const openHtml: string = render(WorkspaceDiffPanel, {
                props: { open: true, conversationId: 'conv-1' },
            }).body;
            assert.ok(openHtml.includes(t(language, 'workspaceWorkflow.panelTitle')));
            assert.ok(openHtml.includes(t(language, 'workspaceWorkflow.undoTurn')));
            assert.ok(openHtml.includes(t(language, 'workspaceWorkflow.redoTurn')));
            assert.ok(openHtml.includes(t(language, 'workspaceWorkflow.tabDiff')));
            assert.ok(openHtml.includes(t(language, 'workspaceWorkflow.tabCheckpoints')));
            assert.ok(openHtml.includes(t(language, 'workspaceWorkflow.tabWorktrees')));
            assert.ok(!openHtml.includes('undefined'));
        }
    } finally {
        await server.close();
    }
});
