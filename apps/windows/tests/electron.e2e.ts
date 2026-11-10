import { _electron as electron, ElectronApplication, expect, Page, test } from "@playwright/test";
import { existsSync, rmSync } from "node:fs";
import path from "node:path";

const windowsRoot = path.resolve(__dirname, "..");
const testUserData = path.join(windowsRoot, "test-results", "profile");
let application: ElectronApplication;
let page: Page;

function requiredExecutable(environmentName: string): string {
    const value = process.env[environmentName];
    if (!value || !existsSync(value)) {
        throw new Error(`${environmentName} must point to a real Windows executable for Electron E2E tests.`);
    }
    return path.resolve(value);
}

test.beforeAll(async () => {
    const cliPath = requiredExecutable("NOLA_WINDOWS_CLI_PATH");
    const llamaServerPath = requiredExecutable("NOLA_LLAMA_SERVER");
    rmSync(testUserData, { recursive: true, force: true });
    application = await electron.launch({
        args: [windowsRoot],
        cwd: windowsRoot,
        env: {
            ...process.env,
            NOLA_WINDOWS_CLI_PATH: cliPath,
            NOLA_LLAMA_SERVER: llamaServerPath,
            NOLA_E2E_USER_DATA: testUserData,
        },
    });
    page = await application.firstWindow();
    await page.waitForURL(/^http:\/\/127\.0\.0\.1:\d+\/chat$/, { timeout: 60_000 });
});

test.afterAll(async () => {
    await application?.close();
});

test("starts the real local Host in a sandboxed desktop renderer", async () => {
    await expect(page.locator("#chatApp")).toBeVisible();
    expect(await page.locator("html").getAttribute("data-nola-desktop")).toBe("windows");
    expect(await application.browserWindow(page).evaluate((window) => ({
        nodeIntegration: window.webContents.getLastWebPreferences().nodeIntegration,
        contextIsolation: window.webContents.getLastWebPreferences().contextIsolation,
        sandbox: window.webContents.getLastWebPreferences().sandbox,
    }))).toEqual({ nodeIntegration: false, contextIsolation: true, sandbox: true });
});

test("uses the Mac-derived three-column chat composition", async () => {
    await page.setViewportSize({ width: 1440, height: 900 });
    const sidebar = await page.locator(".chat-sidebar").boundingBox();
    const main = await page.locator(".chat-main").boundingBox();
    const inspector = await page.locator(".chat-inspector").boundingBox();
    expect(sidebar).not.toBeNull();
    expect(main).not.toBeNull();
    expect(inspector).not.toBeNull();
    expect(sidebar!.width).toBeGreaterThanOrEqual(220);
    expect(main!.x).toBeGreaterThanOrEqual(sidebar!.x + sidebar!.width - 2);
    expect(inspector!.x).toBeGreaterThanOrEqual(main!.x + main!.width - 2);
    expect(inspector!.width).toBeGreaterThanOrEqual(280);
    await page.screenshot({ path: path.join(windowsRoot, "test-results", "artifacts", "chat-three-column.png"), fullPage: true });
});

test("keeps API and llama.cpp models as parallel routes", async () => {
    const origin = new URL(page.url()).origin;
    await page.goto(`${origin}/models`);
    await expect(page.locator(".model-route-grid")).toBeVisible();
    await expect(page.locator(".model-route-section")).toHaveCount(2);
    await expect(page.locator(".engine-dot.ready")).toBeVisible();

    const rail = await page.locator(".navbar").boundingBox();
    const content = await page.locator(".main-content").boundingBox();
    expect(rail).not.toBeNull();
    expect(content).not.toBeNull();
    expect(rail!.width).toBeGreaterThanOrEqual(200);
    expect(content!.x).toBeGreaterThanOrEqual(rail!.width - 2);

    const explicitStartControl = page.getByRole("button", { name: /启动|啟動|Start|起動|시작/i });
    const downloadControl = page.getByRole("button", { name: /下载|下載|Download|ダウンロード|다운로드/i });
    expect((await explicitStartControl.count()) + (await downloadControl.count())).toBeGreaterThan(0);
    await page.screenshot({ path: path.join(windowsRoot, "test-results", "artifacts", "model-routes.png"), fullPage: true });
});
