import { expect, test } from "@playwright/test";
import { spawn, type ChildProcessByStdio } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import type { Readable } from "node:stream";
import { stringsFor } from "../dist/i18n.js";
import { startupPageHtml } from "../dist/startupPage.js";

const repoRoot = path.resolve(__dirname, "../../..");
const webRoot = path.join(repoRoot, "apps/web");
const ALL_LOCALES = ["en", "zh-Hans", "zh-Hant", "ja", "ko", "de", "es", "fr", "ru"] as const;

function parseRgb(raw: string): [number, number, number] {
    const match = raw.match(/rgba?\((\d+),\s*(\d+),\s*(\d+)/);
    if (!match) throw new Error(`Expected RGB color, got ${raw}`);
    return [Number(match[1]), Number(match[2]), Number(match[3])];
}

function relativeLuminance([r, g, b]: [number, number, number]): number {
    const [lr, lg, lb] = [r, g, b].map((c) => {
        const s = c / 255;
        return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * lr + 0.7152 * lg + 0.0722 * lb;
}

function contrastRatio(a: [number, number, number], b: [number, number, number]): number {
    const la = relativeLuminance(a);
    const lb = relativeLuminance(b);
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

function resolveNolaBinary(): string {
    const candidates = [
        process.env.NOLA_WINDOWS_CLI_PATH,
        process.env.NOLA_CLI_PATH,
        path.join(repoRoot, "target/debug", process.platform === "win32" ? "nola.exe" : "nola"),
        path.join(repoRoot, "target/release", process.platform === "win32" ? "nola.exe" : "nola"),
    ].filter((item): item is string => Boolean(item));
    const found = candidates.find((candidate) => fs.existsSync(candidate));
    if (!found) throw new Error(`nola binary not found; checked: ${candidates.join(", ")}`);
    return found;
}

function allocatePort(): Promise<number> {
    return new Promise((resolve, reject) => {
        const server = net.createServer();
        server.unref();
        server.once("error", reject);
        server.listen(0, "127.0.0.1", () => {
            const addr = server.address();
            if (!addr || typeof addr === "string") {
                server.close();
                reject(new Error("Could not allocate loopback port"));
                return;
            }
            const port = addr.port;
            server.close((err) => (err ? reject(err) : resolve(port)));
        });
    });
}

async function waitForHealth(origin: string, child: ChildProcessByStdio<null, Readable, Readable>, logs: string[]): Promise<void> {
    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline) {
        if (child.exitCode !== null) throw new Error(`nola web exited early (code=${child.exitCode}):\n${logs.join("")}`);
        try {
            const res = await fetch(`${origin}/api/health`, { signal: AbortSignal.timeout(1_000) });
            if (res.ok) return;
        } catch {
            // Host still starting.
        }
        await new Promise((resolve) => setTimeout(resolve, 200));
    }
    throw new Error(`Timed out waiting for ${origin}/api/health:\n${logs.join("")}`);
}

async function stopHost(child: ChildProcessByStdio<null, Readable, Readable>): Promise<void> {
    if (child.exitCode !== null || child.signalCode !== null) return;
    child.kill("SIGTERM");
    await Promise.race([
        new Promise<void>((resolve) => child.once("exit", () => resolve())),
        new Promise<void>((resolve) => setTimeout(() => {
            if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
            resolve();
        }, 3_000)),
    ]);
}

test("startupPageHtml enforces Warm Paper/Ink tokens, WCAG contrast, layout bounds, XSS safety, and 9 locales", async ({ browser }) => {
    const hostileDetail = '<script>window.untrustedMarkup=true</script> ' + "Long diagnostic path 中文 ".repeat(24);
    const cases: Array<{
        locale: string;
        scheme: "light" | "dark";
        viewport: { width: number; height: number };
        reducedMotion: "reduce" | "no-preference";
        forcedColors: "active" | "none";
        mode: "starting" | "failed";
        detail: string;
    }> = [];

    for (const locale of ALL_LOCALES) {
        for (const scheme of ["light", "dark"] as const) {
            cases.push({ locale, scheme, viewport: { width: 500, height: 700 }, reducedMotion: "no-preference", forcedColors: "none", mode: "starting", detail: "" });
            cases.push({ locale, scheme, viewport: { width: 320, height: 568 }, reducedMotion: "no-preference", forcedColors: "none", mode: "failed", detail: hostileDetail });
        }
    }
    cases.push({ locale: "en", scheme: "light", viewport: { width: 720, height: 520 }, reducedMotion: "reduce", forcedColors: "none", mode: "starting", detail: "" });
    cases.push({ locale: "zh-Hans", scheme: "dark", viewport: { width: 1440, height: 900 }, reducedMotion: "reduce", forcedColors: "none", mode: "failed", detail: "Short detail" });
    cases.push({ locale: "de", scheme: "light", viewport: { width: 720, height: 520 }, reducedMotion: "no-preference", forcedColors: "active", mode: "failed", detail: "Forced colors test" });

    for (const item of cases) {
        const context = await browser.newContext({
            colorScheme: item.scheme,
            viewport: item.viewport,
            reducedMotion: item.reducedMotion,
            forcedColors: item.forcedColors,
        });
        await context.route("**/*", (route) => route.abort());
        const page = await context.newPage();
        const errors: string[] = [];
        page.on("pageerror", (err) => errors.push(err.message));

        try {
            const strings = stringsFor(item.locale);
            await page.setContent(startupPageHtml(strings, item.mode, item.detail));
            expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
            expect(await page.locator("h1").textContent()).toBe(item.mode === "failed" ? strings.failedTitle : strings.startingTitle);

            if (item.forcedColors !== "active") {
                const expectedBodyBg = item.scheme === "dark" ? "rgb(27, 28, 26)" : "rgb(250, 248, 244)";
                const expectedCardBg = item.scheme === "dark" ? "rgb(34, 36, 32)" : "rgb(255, 255, 255)";
                const styles = await page.evaluate(() => ({
                    bodyBg: getComputedStyle(document.body).backgroundColor,
                    cardBg: getComputedStyle(document.querySelector(".card")!).backgroundColor,
                    h1Fg: getComputedStyle(document.querySelector("h1")!).color,
                    pFg: getComputedStyle(document.querySelector("p")!).color,
                }));
                expect(styles.bodyBg).toBe(expectedBodyBg);
                expect(styles.cardBg).toBe(expectedCardBg);
                const cardRgb = parseRgb(styles.cardBg);
                expect(contrastRatio(parseRgb(styles.h1Fg), cardRgb)).toBeGreaterThanOrEqual(4.5);
                expect(contrastRatio(parseRgb(styles.pFg), cardRgb)).toBeGreaterThanOrEqual(4.5);
            }

            if (item.mode === "failed") {
                expect(await page.locator("script").count()).toBe(0);
                if (item.detail) {
                    expect(await page.locator(".detail").textContent()).toBe(item.detail);
                }
                const actionsBottom = await page.locator(".actions").evaluate((el) => el.getBoundingClientRect().bottom);
                expect(actionsBottom).toBeLessThanOrEqual(item.viewport.height);
                expect(await page.locator("#nola-open-logs").textContent()).toBe(strings.openLogs);
                expect(await page.locator("#nola-retry-host").textContent()).toBe(strings.retry);
            } else if (item.reducedMotion === "reduce") {
                expect(await page.locator(".pulse").evaluate((el) => getComputedStyle(el, "::after").animationName)).toBe("none");
            }
            expect(errors).toEqual([]);
        } finally {
            await context.close();
        }
    }
});

// Exercise the production bridge and startup page with a controlled Tauri command transport.
const bridge = fs.readFileSync(path.join(repoRoot, "apps/windows/src-tauri/src/desktop-bridge.js"), "utf8");
const startup = fs.readFileSync(path.join(repoRoot, "apps/windows/src-tauri/startup/startup.js"), "utf8");
const preloadHarness = `(() => {
    let enabled = false, reads = 0;
    window.__ipcCalls = [];
    window.__TAURI_INTERNALS__ = {invoke: async (method, args) => {
        if (method === "startup_status") return {phase:"failed",language:"en",failure:"Exit code 1",page:${JSON.stringify(startupPageHtml(stringsFor("en"), "failed", "DETAIL"))}};
        window.__ipcCalls.push(method);
        switch (method) {
            case "retry_startup": return new Promise(resolve => { window.__finishRetry = resolve; });
            case "native_runtime": return {architecture:"x64", backend:"vulkan"};
            case "login_item":
                if (++reads === 1) throw new Error("Disconnected");
                return {available:true, enabled, labels:{title:"Start Nola when I sign in",body:"",enabled:"Enabled",disabled:"Disabled",unavailable:"Unavailable"}};
            case "set_login_item": enabled = args.enabled; throw new Error("Lost reply");
            case "set_theme": return null;
            case "open_diagnostics": return true;
            default: throw new Error("Unexpected IPC " + method);
        }
    }};
    ${bridge}
})();`;

test("startup recovery buttons enforce Tauri command and busy/retry guards", async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width: 720, height: 520 }, colorScheme: "light" });
    const index = fs.readFileSync(path.join(repoRoot, "apps/windows/src-tauri/startup/index.html"), "utf8");
    await context.route("http://tauri.localhost/**", route => route.request().url().endsWith("/startup.js")
        ? route.fulfill({contentType:"text/javascript", body:startup})
        : route.fulfill({contentType:"text/html", body:index}));
    const page = await context.newPage();
    try {
        await page.addInitScript({ content: preloadHarness });
        await page.goto("http://tauri.localhost/index.html");
        await page.locator("#nola-open-logs").click();
        await page.locator("#nola-retry-host").click();
        await expect(page.locator("#nola-retry-host")).toBeDisabled();
        await expect(page.locator("#nola-retry-host")).toHaveAttribute("aria-busy", "true");

        // Second click while busy must not re-invoke IPC.
        await page.locator("#nola-retry-host").click({ force: true });
        expect(await page.evaluate(() => (window as unknown as { __ipcCalls: string[] }).__ipcCalls)).toEqual([
            "open_diagnostics",
            "retry_startup",
        ]);

        // Rejected / false retry re-enables button and clears aria-busy.
        await page.evaluate(() => (window as unknown as { __finishRetry: (ok: boolean) => void }).__finishRetry(false));
        await expect(page.locator("#nola-retry-host")).toBeEnabled();
        expect(await page.locator("#nola-retry-host").getAttribute("aria-busy")).toBeNull();
    } finally {
        await context.close();
    }
});

test("startup recovers from a missed status read and shows native action errors", async ({ page }) => {
    await page.setContent("<main>Nola</main>");
    await page.addScriptTag({content: preloadHarness});
    await page.addScriptTag({content: `{
        const original = window.__TAURI_INTERNALS__.invoke;
        let unavailable = true;
        window.__TAURI_INTERNALS__.invoke = (command, args) => {
            if (command === "startup_status" && unavailable) {
                unavailable = false;
                return Promise.reject(new Error("Bridge connecting"));
            }
            if (command === "open_diagnostics") return new Promise((_, reject) => {
                window.__rejectDiagnostics = () => reject(new Error("Folder unavailable"));
            });
            return original(command, args);
        };
    }`});
    await page.addScriptTag({content: startup});
    const logs = page.getByRole("button", {name: stringsFor("en").openLogs});
    await logs.focus();
    await logs.press("Enter");
    await expect(logs).toBeDisabled();
    const rejectDiagnostics = () => page.evaluate(() =>
        (window as Window & { __rejectDiagnostics(): void }).__rejectDiagnostics());
    await rejectDiagnostics();
    await expect(page.getByRole("status")).toHaveText("Folder unavailable");
    await expect(logs).toBeEnabled();
    await expect(logs).toBeFocused();
    await expect(page.locator(".detail")).toHaveText("Exit code 1");
    await expect(page).toHaveTitle(stringsFor("en").failedTitle);
    await logs.press("Enter");
    await expect(logs).toBeDisabled();
    const retry = page.locator("#nola-retry-host");
    await retry.focus();
    await rejectDiagnostics();
    await expect(logs).toBeEnabled();
    await expect(retry).toBeFocused();
});

test("Windows desktop shell integration against isolated real nola web renders Svelte 5 chat, Settings login card, and /models runtime badge", async ({ browser }) => {
    const cliPath = resolveNolaBinary();
    const port = await allocatePort();
    const origin = `http://127.0.0.1:${port}`;
    const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "nola-win-shell-"));
    const tmpHome = path.join(tmpRoot, "home");
    const tmpDb = path.join(tmpRoot, "nola.db");
    fs.mkdirSync(tmpHome, { recursive: true });

    const childEnv: NodeJS.ProcessEnv = {
        ...process.env,
        HOME: tmpHome,
        NOLA_HOME: tmpHome,
        NOLA_DB_PATH: tmpDb,
        NOLA_DESKTOP_PLATFORM: "windows",
    };
    delete childEnv.NOLA_MAC_APP_SUPPORT;

    const logs: string[] = [];
    const host = spawn(cliPath, ["web", "--host", "127.0.0.1", "--port", String(port), "--web-root", webRoot], {
        cwd: tmpRoot,
        env: childEnv,
        stdio: ["ignore", "pipe", "pipe"],
    });
    host.stdout.on("data", (chunk: Buffer) => logs.push(chunk.toString("utf8")));
    host.stderr.on("data", (chunk: Buffer) => logs.push(chunk.toString("utf8")));

    try {
        await waitForHealth(origin, host, logs);
        const context = await browser.newContext({ viewport: { width: 1280, height: 840 }, colorScheme: "dark", locale: "en-US" });
        await context.addInitScript({ content: preloadHarness });

        const page = await context.newPage();
        const pageErrors: string[] = [];
        page.on("pageerror", (err) => pageErrors.push(err.message));

        // Record the real viewport after each postcondition. The report retains
        // the last visible state even when a click or its postcondition fails.
        async function action(name: string, perform: () => Promise<void>, verify: () => Promise<void>) {
            await test.step(name, async () => {
                async function capture(phase: string) {
                    const filename = name + '-' + phase + '.png';
                    const image = test.info().outputPath(filename);
                    await page.screenshot({path: image, animations: "disabled"});
                    await test.info().attach(filename, {path: image, contentType: "image/png"});
                }
                await capture('before');
                try {
                    await perform();
                    await verify();
                    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
                } catch (error) {
                    await capture('failure');
                    throw error;
                }
                await capture('after');
            });
        }

        await page.goto(origin + '/chat');
        await expect(page.locator('html')).toHaveAttribute('data-nola-desktop', 'windows');
        await expect(page.locator('.chat-main')).toBeVisible();
        expect(await page.evaluate(() => getComputedStyle(document.body).backgroundColor)).toBe('rgb(27, 28, 26)');
        const composer = page.locator('#chat-composer');
        const settings = page.locator('.settings-overview');
        const settingsSidebar = page.locator('.settings-sidebar');
        const card = page.locator('[id$="desktopLoginItemCard"]');
        const toggle = card.locator('input[role="switch"]');
        await action('01-create-conversation', () => page.getByRole('button', {name:'New conversation', exact:true}).click(),
            async () => {
                await expect(composer).toBeEnabled();
                await expect(page.getByRole('tab', {name:'New conversation', exact:true})).toHaveAttribute('aria-selected','true');
            });
        await action('02-enter-draft', () => composer.fill('Windows desktop shell roundtrip'),
            () => expect(composer).toHaveValue('Windows desktop shell roundtrip'));
        const owner = new URL(page.url()).searchParams.get('conversation');
        expect(owner).toBeTruthy();
        const conversations = await page.request.get(origin + '/api/chat/conversations');
        expect(conversations.ok()).toBe(true);
        expect((await conversations.json()).conversations.map((value: {conversationId: string}) => value.conversationId)).toEqual([owner]);
        await action('send-without-configured-model', () => page.getByRole('button', {name:'Send', exact:true}).click(),
            async () => {
                await expect(page.getByRole('link', {name:'Add a chat model'})).toBeVisible();
                await expect(composer).toHaveValue('Windows desktop shell roundtrip');
            });
        await action('03-open-settings', () => page.getByRole('button', {name:'Settings', exact:true}).click(),
            () => expect(settingsSidebar).toBeVisible());
        const categories = [
            ['general', 'General'], ['agent', 'Agent'], ['connections', 'Connections'],
            ['data', 'Data'], ['diagnostics', 'Diagnostics'], ['providers', 'API Providers'],
        ];
        async function categoryReady(id: string) {
            const panel = page.locator('[id$="settings-panel-' + id + '"]');
            await expect(panel).toBeVisible();
            await expect(panel.getByRole('heading').first()).toBeVisible();
            if (id === 'diagnostics') await expect(panel.locator('dl')).toBeVisible();
            if (id === 'agent') await expect(panel.locator('.agent-access fieldset')).toBeVisible();
        }
        for (const [id, label] of categories) {
            const tab = page.getByRole('tab', {name:label, exact:true});
            await action('desktop-' + id, () => tab.click(), async () => {
                await expect(tab).toHaveAttribute('aria-selected','true');
                await categoryReady(id);
            });
            if (id === 'general') {
                await expect(card.getByRole('alert')).toBeVisible();
                await action('retry-login-setting', () => card.getByRole('button').click(),
                    () => expect(toggle).not.toBeChecked());
                await action('enable-login-setting', () => toggle.click(), async () => {
                    await expect(toggle).toBeChecked();
                    await expect(card.getByRole('alert')).toHaveCount(0);
                });
            }
        }
        await action('open-models', () => page.getByRole('link', {name:'Manage API models'}).click(),
            () => expect(page.locator('[role="tab"][id$="tab-api"]')).toHaveAttribute('aria-selected','true'));
        await action('show-local-models', () => page.locator('[role="tab"][id$="tab-local"]').click(), async () => {
            await expect(page.locator('[id$="panel-local"]')).toBeVisible();
            await expect(page.locator('[data-local-runtime-host] .desktop-native-runtime')).toHaveText('VULKAN · X64');
        });

        await page.setViewportSize({width:390,height:844});
        // Re-enter the settings workspace, then use only the visible narrow-screen
        // category and back controls to traverse all six sections.
        await page.goto(origin + '/settings');
        for (const [id,label] of categories) {
            const link = settings.getByRole('link',{name:label, exact:true});
            await action('narrow-open-' + id, () => link.click(), async () => {
                await categoryReady(id);
                await expect(settings).toBeHidden();
            });
            await action('narrow-back-' + id, () => page.locator('.settings-back button:visible').click(),
                () => expect(link).toBeVisible());
        }

        expect(pageErrors).toEqual([]);
        await context.close();
    } finally {
        await stopHost(host);
        fs.rmSync(tmpRoot, { recursive: true, force: true });
    }
});
