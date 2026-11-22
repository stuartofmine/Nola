// SPDX-License-Identifier: Apache-2.0
// CDP harness for the instrumented real-model acceptance run.
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { setTimeout: delay } = require("node:timers/promises");
const { chromium } = require("playwright-core");

async function launchDesktop(executable, home) {
    const child = spawn(executable, [], { env: { ...process.env, NOLA_HOME: home,
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=19223" } });
    const exit = once(child, "exit");
    let browser;
    let lastError;
    try {
        for (let attempt = 0; attempt < 120; attempt++) {
            assert.equal(child.exitCode, null, "Installed Tauri process exited");
            try {
                const res = await fetch("http://127.0.0.1:19223/json/version");
                if (res.ok) {
                    browser = await chromium.connectOverCDP((await res.json()).webSocketDebuggerUrl, { noDefaults:true, timeout:3000 });
                    break;
                }
            } catch (error) { lastError = error; }
            await delay(500);
        }
        assert.ok(browser, "WebView2 debugging session did not open: " + (lastError?.message ?? "no debugger endpoint"));
        const context = browser.contexts()[0];
        const page = context.pages()[0] ?? await context.waitForEvent("page");
        try {
            await page.waitForURL(/^http:\/\/127\.0\.0\.1:\d+\/chat/, { timeout: 65000 });
        } catch (error) {
            console.error("Startup page:", (await page.locator("body").innerText()).slice(0, 600));
            throw error;
        }
        return {page, close: async () => {
            child.kill();
            await exit;
            if (browser) await browser.close().catch(() => {});
        }};
    } catch (error) {
        child.kill();
        await exit;
        if (browser) await browser.close().catch(() => {});
        throw error;
    }
}

module.exports = { launchDesktop };
