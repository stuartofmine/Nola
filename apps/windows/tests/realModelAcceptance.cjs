// SPDX-License-Identifier: Apache-2.0
// Started only by scripts/real-model-acceptance.py with an isolated Host/relay.
const assert = require('node:assert/strict');
const { readFileSync, writeFileSync } = require('node:fs');
const path = require('node:path');
const { _electron: electron, expect } = require('@playwright/test');

async function main() {
    const evidence = process.env.NOLA_ELECTRON_MODEL_EVIDENCE;
    assert.ok(evidence && path.isAbsolute(evidence) && process.env.NOLA_E2E_USER_DATA, 'Use the real-model acceptance runner');
    const windows = path.resolve(__dirname, '..');
    const receipt = { os: process.platform, client: 'windows-electron', passed: false };
    const workspace = path.join(process.env.NOLA_E2E_USER_DATA, 'Nola Workspace');
    const expected = readFileSync(path.join(workspace, 'handoff-note.txt'), 'utf8').trim();
    const question = 'Read handoff-note.txt in this workspace with the read_file tool. Reply with only the reference code inside. Leave all files unchanged.';
    const timeline = [];
    function writeJson(name, value) {
        writeFileSync(path.join(evidence, name), JSON.stringify(value, null, 2));
    }
    let application, page;
    async function launch() {
        application = await electron.launch({ args: [windows], cwd: windows, env: process.env, timeout: 45000 });
        page = await application.firstWindow();
        await page.waitForURL(url => url.origin.startsWith('http://127.0.0.1:') && url.pathname === '/chat', {timeout: 60000});
        await expect(page.locator('html')).toHaveAttribute('data-nola-desktop', 'windows');
    }
    const answer = () => page.locator('article:not(.user) .message-content').getByText(expected, {exact: true});
    try {
        await launch();
        receipt.origin = new URL(page.url()).origin;
        await page.locator('button.new-conversation:visible').first().click();
        await page.waitForURL(url => Boolean(url.searchParams.get('conversation')));
        receipt.conversation = new URL(page.url()).searchParams.get('conversation');
        await page.locator('#chat-composer').fill(question);
        await page.evaluate(() => {
            const samples = [];
            let last;
            function capture() {
                const running = !!document.querySelector('.composer-actions button[type=button]');
                const assistantText = [...document.querySelectorAll('article:not(.user) .message-content .markdown')]
                    .map(node => node.innerText).join('\n');
                if (last?.running === running && last.assistantText === assistantText) return;
                last = {stage: 'send', time: Date.now(), running, assistantText, assistantCharacters: assistantText.length};
                samples.push(last);
            }
            capture();
            new MutationObserver(capture).observe(document.body, {subtree: true, childList: true, characterData: true});
            window.nolaAcceptanceSamples = samples;
        });
        await page.locator('form.composer button[type=submit]').click();
        await expect(answer()).toBeVisible({timeout: 180000});
        // The rendered final-text part can appear just before the Run is terminal.
        await expect(page.locator('form.composer button[type=submit]')).toBeVisible({timeout: 30000});
        await expect(page.locator('.composer-actions button[type=button]')).toHaveCount(0);
        timeline.push(...await page.evaluate(() => window.nolaAcceptanceSamples));
        assert.ok(timeline.some(row => row.running), 'Renderer never exposed an active turn');
        assert.equal(timeline.at(-1).running, false);
        receipt.expectedToken = expected;
        receipt.response = await answer().innerText();
        await page.screenshot({path: path.join(evidence, 'submitted.png')});
        await application.close();
        application = undefined;
        await launch();
        assert.equal(new URL(page.url()).origin, receipt.origin);
        if (new URL(page.url()).searchParams.get('conversation') !== receipt.conversation) {
            // This profile has exactly one conversation. Reopen it from the real sidebar.
            const rows = page.locator('.chat-navigation button.conversation:visible');
            await expect(rows).toHaveCount(1);
            await rows.click();
        }
        await page.waitForURL(url => url.searchParams.get('conversation') === receipt.conversation);
        await expect(answer()).toBeVisible({timeout: 30000});
        await page.screenshot({path: path.join(evidence, 'restored.png')});
        receipt.restoredReply = await answer().innerText();
        const userText = await page.locator('article.user .message-content').getByText(question, {exact: true}).innerText();
        writeJson('transcript.json', [{role: 'user', text: userText}, {role: 'assistant', text: receipt.restoredReply}]);
        const tree = await page.locator('body').ariaSnapshot();
        assert.ok(tree.includes(expected));
        writeFileSync(path.join(evidence, 'accessibility.txt'), tree);
        timeline.push({stage: 'restored', time: Date.now(),
            running: await page.locator('.composer-actions button[type=button]').count() > 0,
            assistantText: receipt.restoredReply, assistantCharacters: receipt.restoredReply.length});
        assert.equal(timeline.at(-1).running, false);
        writeJson('timeline.json', timeline);
        receipt.restored = true;
        receipt.passed = true;
    } finally {
        writeFileSync(path.join(evidence, 'renderer.json'), JSON.stringify(receipt, null, 2));
        try { if (!receipt.passed && page && !page.isClosed()) await page.screenshot({path: path.join(evidence, 'failure.png')}); }
        finally { if (application) await application.close(); }
    }
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
