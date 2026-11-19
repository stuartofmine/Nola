import { _electron as electron, ElectronApplication, expect, Page, test } from "@playwright/test";
import { existsSync, rmSync } from "node:fs";
import path from "node:path";

test.skip(
    process.platform !== "win32" && process.env.NOLA_RUN_ELECTRON_E2E !== "1",
    "Windows Electron shell E2E runs on Windows or when NOLA_RUN_ELECTRON_E2E=1",
);

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
    await expect(page.locator(".chat-page")).toBeVisible();
    expect(await page.locator("html").getAttribute("data-nola-desktop")).toBe("windows");
    expect(await application.browserWindow(page).evaluate((window) => ({
        nodeIntegration: window.webContents.getLastWebPreferences().nodeIntegration,
        contextIsolation: window.webContents.getLastWebPreferences().contextIsolation,
        sandbox: window.webContents.getLastWebPreferences().sandbox,
    }))).toEqual({ nodeIntegration: false, contextIsolation: true, sandbox: true });
});

test("uses the Svelte 5 workspace composition on desktop", async () => {
    await page.setViewportSize({ width: 1440, height: 900 });
    const navigation = await page.locator(".chat-navigation").boundingBox();
    const main = await page.locator(".chat-main").boundingBox();
    expect(navigation).not.toBeNull();
    expect(main).not.toBeNull();
    expect(navigation!.width).toBeGreaterThanOrEqual(220);
    expect(main!.x).toBeGreaterThanOrEqual(navigation!.x + navigation!.width - 2);
    await expect(page.locator("button.new-conversation:visible").first()).toBeVisible();
    await page.screenshot({ path: path.join(windowsRoot, "test-results", "artifacts", "chat-desktop-shell.png"), fullPage: true });
});

test("keeps API and local llama.cpp models accessible with native runtime badge", async () => {
    const origin = new URL(page.url()).origin;
    await page.goto(`${origin}/models`);
    const apiTab = page.locator('[role="tab"][id$="tab-api"]');
    const localTab = page.locator('[role="tab"][id$="tab-local"]');
    await expect(apiTab).toBeVisible();
    await expect(localTab).toBeVisible();

    await localTab.click();
    await expect(page.locator('[id$="panel-local"]')).toBeVisible();
    await expect(page.locator("[data-local-runtime-host]")).toBeVisible();
    await expect(page.locator("[data-local-runtime-host] .desktop-native-runtime")).toBeVisible();

    const startControl = page.getByRole("button", { name: /启动|啟動|Start|起動|시작|Starten|Iniciar|Démarrer|Запустить/i });
    const downloadControl = page.getByRole("button", { name: /下载|下載|Download|ダウンロード|다운로드|Herunterladen|Descargar|Télécharger|Скачать/i });
    expect((await startControl.count()) + (await downloadControl.count())).toBeGreaterThan(0);
    await page.screenshot({ path: path.join(windowsRoot, "test-results", "artifacts", "model-routes.png"), fullPage: true });
});
