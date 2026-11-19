// SPDX-License-Identifier: Apache-2.0
// Run on an isolated Windows account after installing the NSIS package.
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { once } = require("node:events");
const { setTimeout: delay } = require("node:timers/promises");
const { chromium } = require("playwright-core");

async function withDesktop(executable, home, inspect) {
    const child = spawn(executable, [], { env: { ...process.env, NOLA_HOME: home,
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=19223" } });
    const exit = once(child, "exit");
    let browser;
    try {
        for (let attempt = 0; attempt < 120; attempt++) {
            assert.equal(child.exitCode, null, "Installed Tauri process exited");
            try {
                const res = await fetch("http://127.0.0.1:19223/json/version");
                if (res.ok) {
                    browser = await chromium.connectOverCDP((await res.json()).webSocketDebuggerUrl);
                    break;
                }
            } catch {}
            await delay(500);
        }
        assert.ok(browser, "WebView2 debugging session did not open");
        const context = browser.contexts()[0];
        const page = context.pages()[0] ?? await context.waitForEvent("page");
        try {
            await page.waitForURL(/^http:\/\/127\.0\.0\.1:\d+\/chat/, { timeout: 65000 });
        } catch (error) {
            console.error("Startup page:", (await page.locator("body").innerText()).slice(0, 600));
            throw error;
        }
        return await inspect(page, new URL(page.url()).origin);
    } finally {
        child.kill();
        await exit;
        if (browser) await browser.close().catch(() => {});
    }
}

async function main() {
    const executable = process.env.NOLA_INSTALLED_DESKTOP;
    assert.ok(executable && fs.existsSync(executable), "Set NOLA_INSTALLED_DESKTOP to the installed Tauri executable");
    const home = fs.mkdtempSync(path.join(os.tmpdir(), "nola-desktop-test-"));
    const created = await withDesktop(executable, home, async (page, origin) => {
        const button = page.locator("button.new-conversation:visible").first();
        const [result] = await Promise.all([
            page.waitForResponse(r => r.url().endsWith("/api/chat/conversations") && r.request().method() === "POST"),
            button.click(),
        ]);
        assert.ok(result.ok(), "Create conversation must commit");
        const value = await result.json();
        const id = value.conversationId ?? value.conversation?.conversationId;
        assert.ok(id);
        await page.locator("#chat-composer").fill("Tauri desktop verification");
        assert.equal(await page.locator("#chat-composer").inputValue(), "Tauri desktop verification");
        return { id, origin };
    });
    await withDesktop(executable, home, async (page, origin) => {
        assert.equal(origin, created.origin, "Desktop origin must survive relaunch");
        const response = await page.request.get(origin + "/api/chat/conversations");
        assert.ok(response.ok());
        assert.ok((await response.json()).conversations.some(c => c.conversationId === created.id),
            "The conversation created in the installed UI must survive relaunch");
    });
    console.log("Installed Tauri/WebView2 client passed launch, conversation creation, composer and restart.");
}
main().catch(error => { console.error(error); process.exitCode = 1; });
