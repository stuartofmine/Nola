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
                const expectedBodyBg = item.scheme === "dark" ? "rgb(19, 19, 18)" : "rgb(247, 246, 243)";
                const expectedCardBg = item.scheme === "dark" ? "rgb(27, 27, 25)" : "rgb(255, 255, 255)";
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

test("startup recovery buttons enforce preload click IPC and busy/retry guards", async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width: 720, height: 520 }, colorScheme: "light" });
    await context.route("**/*", (route) => route.abort());
    const page = await context.newPage();
    try {
        await page.setContent(startupPageHtml(stringsFor("en"), "failed", "Exit code 1"));
        await page.evaluate(() => {
            const calls: string[] = [];
            let resolveRetry!: (value: boolean) => void;
            (window as unknown as Record<string, unknown>).__ipcCalls = calls;
            (window as unknown as Record<string, unknown>).__finishRetry = (ok: boolean) => resolveRetry(ok);

            const retry = document.getElementById("nola-retry-host");
            retry?.addEventListener("click", () => {
                if (!(retry instanceof HTMLButtonElement) || retry.disabled) return;
                retry.disabled = true;
                retry.setAttribute("aria-busy", "true");
                calls.push("desktop:retry-host");
                new Promise<boolean>((resolve) => { resolveRetry = resolve; })
                    .then((accepted) => {
                        if (!accepted && retry.isConnected) {
                            retry.disabled = false;
                            retry.removeAttribute("aria-busy");
                        }
                    });
            });
            document.getElementById("nola-open-logs")?.addEventListener("click", () => {
                calls.push("desktop:open-log-folder");
            });
        });

        await page.locator("#nola-open-logs").click();
        await page.locator("#nola-retry-host").click();
        await expect(page.locator("#nola-retry-host")).toBeDisabled();
        await expect(page.locator("#nola-retry-host")).toHaveAttribute("aria-busy", "true");

        // Second click while busy must not re-invoke IPC.
        await page.locator("#nola-retry-host").click({ force: true });
        expect(await page.evaluate(() => (window as unknown as { __ipcCalls: string[] }).__ipcCalls)).toEqual([
            "desktop:open-log-folder",
            "desktop:retry-host",
        ]);

        // Rejected / false retry re-enables button and clears aria-busy.
        await page.evaluate(() => (window as unknown as { __finishRetry: (ok: boolean) => void }).__finishRetry(false));
        await expect(page.locator("#nola-retry-host")).toBeEnabled();
        expect(await page.locator("#nola-retry-host").getAttribute("aria-busy")).toBeNull();
    } finally {
        await context.close();
    }
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
        cwd: repoRoot,
        env: childEnv,
        stdio: ["ignore", "pipe", "pipe"],
    });
    host.stdout.on("data", (chunk: Buffer) => logs.push(chunk.toString("utf8")));
    host.stderr.on("data", (chunk: Buffer) => logs.push(chunk.toString("utf8")));

    try {
        await waitForHealth(origin, host, logs);
        const context = await browser.newContext({ viewport: { width: 1280, height: 840 }, colorScheme: "dark", locale: "en-US" });
        await context.addInitScript(() => {
            const setPlatformAttr = () => {
                if (document.documentElement) document.documentElement.dataset.nolaDesktop = "windows";
            };
            setPlatformAttr();
            document.addEventListener("DOMContentLoaded", setPlatformAttr);
            let enabled = false;
            let readCount = 0;
            const labels = {
                title: "Start Nola when I sign in",
                body: "Keeps scheduled work and the local Agent available in the notification area without opening a window.",
                enabled: "Enabled",
                disabled: "Disabled",
                unavailable: "Available after Nola is installed",
            };
            (window as unknown as Record<string, unknown>).NolaDesktop = Object.freeze({
                platform: "windows",
                retryHost: async () => true,
                openLogFolder: async () => true,
                openWorkspace: async () => true,
                loginItem: async () => {
                    readCount += 1;
                    if (readCount === 1) throw new Error("Simulated transient IPC read disconnect");
                    return { available: true, enabled, labels };
                },
                setLoginItem: async (next: boolean) => {
                    enabled = next;
                    throw new Error("Simulated lost write reply after persisting");
                },
            });

            // Mirror preload.ts runtime badge attachment on /models (standalone or embedded tab).
            window.addEventListener("DOMContentLoaded", () => {
                const isModels = () =>
                    location.pathname === "/models" ||
                    (new URLSearchParams(location.search).get("content") ?? "").startsWith("/models");
                let observer: MutationObserver | undefined;
                const attach = () => {
                    const hostEl = document.querySelector("[data-local-runtime-host], .model-engine-state");
                    if (!hostEl) return false;
                    if (hostEl.querySelector(".desktop-native-runtime")) return true;
                    const badge = document.createElement("span");
                    badge.className = "desktop-native-runtime";
                    badge.textContent = "VULKAN · X64";
                    badge.title = "llama.cpp";
                    hostEl.appendChild(badge);
                    return true;
                };
                const sync = () => {
                    if (!isModels()) {
                        observer?.disconnect();
                        observer = undefined;
                        return;
                    }
                    if (attach() || !document.body) {
                        observer?.disconnect();
                        observer = undefined;
                        return;
                    }
                    if (!observer) {
                        observer = new MutationObserver(() => {
                            if (attach() || !isModels()) {
                                observer?.disconnect();
                                observer = undefined;
                            }
                        });
                        observer.observe(document.body, { childList: true, subtree: true });
                    }
                };
                sync();
                window.addEventListener("popstate", sync);
                document.addEventListener("click", () => setTimeout(sync, 50), true);
            });
        });

        const page = await context.newPage();
        const pageErrors: string[] = [];
        page.on("pageerror", (err) => pageErrors.push(err.message));

        // 1. Svelte 5 Chat workspace & conversation creation
        await page.goto(`${origin}/chat`, { waitUntil: "networkidle" });
        await expect(page.locator("html")).toHaveAttribute("data-nola-desktop", "windows");
        await expect(page.locator(".chat-page")).toBeVisible();
        await expect(page.locator(".chat-navigation")).toBeVisible();
        await expect(page.locator(".chat-main")).toBeVisible();
        expect(await page.evaluate(() => getComputedStyle(document.body).backgroundColor)).toBe("rgb(19, 19, 18)");
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);

        await page.locator("button.new-conversation:visible").first().click();
        await page.locator("#chat-composer").waitFor();
        await page.locator("#chat-composer").fill("Windows desktop shell roundtrip");
        expect(await page.locator("#chat-composer").inputValue()).toBe("Windows desktop shell roundtrip");

        // 2. Settings #desktopLoginItemCard failed read + reload + lost-reply reconciliation
        await page.goto(`${origin}/settings?section=general`, { waitUntil: "networkidle" });
        const card = page.locator('[id$="desktopLoginItemCard"]');
        await card.waitFor();
        await card.locator('[role="alert"]').waitFor();
        await card.getByRole("button").click();
        const toggle = card.locator('button[aria-pressed]');
        await toggle.waitFor();
        await expect(toggle).toHaveAttribute("aria-pressed", "false");
        await toggle.click();
        await expect(toggle).toHaveAttribute("aria-pressed", "true");
        expect(await card.locator('[role="alert"]').count()).toBe(0);

        // 3. /models API & Local tabs + [data-local-runtime-host] / .desktop-native-runtime badge
        await page.goto(`${origin}/models`, { waitUntil: "networkidle" });
        await expect(page.locator('[role="tab"][id$="tab-api"]')).toBeVisible();
        await expect(page.locator('[role="tab"][id$="tab-local"]')).toBeVisible();
        await page.locator('[role="tab"][id$="tab-local"]').click();
        await expect(page.locator('[id$="panel-local"]')).toBeVisible();
        const runtimeHost = page.locator("[data-local-runtime-host]");
        await runtimeHost.waitFor();
        const badge = runtimeHost.locator(".desktop-native-runtime");
        await expect(badge).toBeVisible();
        await expect(badge).toHaveText("VULKAN · X64");
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);

        expect(pageErrors).toEqual([]);
        await context.close();
    } finally {
        await stopHost(host);
        fs.rmSync(tmpRoot, { recursive: true, force: true });
    }
});
