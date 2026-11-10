import { app, dialog, BrowserWindow, ipcMain, Menu, nativeImage, nativeTheme, screen, session, shell, Tray } from "electron";
import { createWriteStream, existsSync, mkdirSync, readFileSync, renameSync, writeFileSync, WriteStream } from "node:fs";
import path from "node:path";
import { HostController, HostState, resolveExecutable } from "./host";
import { DesktopStrings, stringsFor } from "./i18n";
import { isSoftwareUpdateDownload } from "./softwareUpdate";
import { restoreWindowGeometry, SavedWindowState } from "./windowGeometry";
import { startupPageHtml } from "./startupPage";

let mainWindow: BrowserWindow | undefined;
let tray: Tray | undefined;
let host: HostController | undefined;
let hostOrigin: string | undefined;
let logStream: WriteStream | undefined;
let quitting = false;
let recoveryTimer: NodeJS.Timeout | undefined;
let automaticRecoveryCount = 0;
let restartingHost = false;
let strings: DesktopStrings;
let explainedBackgroundMode = false;
let windowStateTimer: NodeJS.Timeout | undefined;
const debugEnabled = process.argv.includes("--debug") || process.env.NOLA_DEBUG === "1";
const launchInBackground = process.argv.includes("--background");

type NativeRuntimeInfo = {
    architecture: string;
    backend: string;
    llamaCppRelease?: string;
    llamaCppCommit?: string;
};

const repoRoot = path.resolve(__dirname, "../..");
const executableName = process.platform === "win32" ? "nola.exe" : "nola";
const agentHostName = process.platform === "win32" ? "nola-agent-host.exe" : "nola-agent-host";
const llamaServerName = process.platform === "win32" ? "llama-server.exe" : "llama-server";

if (process.env.NOLA_E2E_USER_DATA) {
    app.setPath("userData", path.resolve(process.env.NOLA_E2E_USER_DATA));
}

function resourcesRoot(): string {
    return app.isPackaged ? process.resourcesPath : repoRoot;
}

function loadNativeRuntimeInfo(): NativeRuntimeInfo {
    const manifest = app.isPackaged
        ? path.join(process.resourcesPath, "native-runtime.json")
        : path.join(repoRoot, "windows", "resources", "native-runtime.json");
    if (existsSync(manifest)) {
        try {
            return JSON.parse(readFileSync(manifest, "utf8")) as NativeRuntimeInfo;
        } catch (error) {
            log(`[runtime] Could not read ${manifest}: ${error instanceof Error ? error.message : String(error)}`);
        }
    }
    return {
        architecture: process.arch,
        backend: process.env.NOLA_LLAMA_BACKEND ?? (process.env.NOLA_LLAMA_SERVER ? "external" : "path"),
    };
}

function windowStatePath(): string {
    return path.join(app.getPath("userData"), "window-state.json");
}

function loadWindowState(): SavedWindowState {
    let saved: Partial<SavedWindowState> = {};
    const file = windowStatePath();
    if (existsSync(file)) {
        try {
            const parsed: unknown = JSON.parse(readFileSync(file, "utf8"));
            if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) saved = parsed as Partial<SavedWindowState>;
        } catch (error) {
            log(`[window] Could not restore state: ${error instanceof Error ? error.message : String(error)}`);
        }
    }
    return restoreWindowGeometry(saved, screen.getAllDisplays().map(display => display.workArea), screen.getPrimaryDisplay().workArea);
}

function persistWindowState(window: BrowserWindow): void {
    if (window.isDestroyed() || window.isMinimized()) return;
    const bounds = window.isMaximized() ? window.getNormalBounds() : window.getBounds();
    const state: SavedWindowState = { ...bounds, maximized: window.isMaximized() };
    const file = windowStatePath();
    const temporary = `${file}.tmp`;
    try {
        writeFileSync(temporary, `${JSON.stringify(state, null, 2)}\n`, "utf8");
        renameSync(temporary, file);
    } catch (error) {
        log(`[window] Could not persist state: ${error instanceof Error ? error.message : String(error)}`);
    }
}

function scheduleWindowStateSave(window: BrowserWindow): void {
    if (windowStateTimer) clearTimeout(windowStateTimer);
    windowStateTimer = setTimeout(() => persistWindowState(window), 220);
}

function desktopPaths() {
    const resources = resourcesRoot();
    const workspace = path.join(app.getPath("documents"), "Nola Workspace");
    const logDirectory = path.join(app.getPath("userData"), "logs");
    mkdirSync(workspace, { recursive: true });
    mkdirSync(logDirectory, { recursive: true });

    const cliPath = resolveExecutable({
        override: process.env.NOLA_WINDOWS_CLI_PATH,
        packagedCandidate: path.join(resources, "bin", executableName),
        developmentCandidates: [
            path.join(repoRoot, "target", "debug", executableName),
            path.join(repoRoot, "target", "release", executableName),
            path.join(repoRoot, "target", "x86_64-pc-windows-msvc", "debug", executableName),
            path.join(repoRoot, "target", "x86_64-pc-windows-msvc", "release", executableName),
            path.join(repoRoot, "target", "aarch64-pc-windows-msvc", "release", executableName),
        ],
    });
    const llamaServerPath = resolveExecutable({
        override: process.env.NOLA_LLAMA_SERVER,
        packagedCandidate: path.join(resources, "bin", llamaServerName),
        developmentCandidates: [
            path.join(repoRoot, "vendor", "llama.cpp", "build", "bin", "Release", llamaServerName),
            path.join(repoRoot, "vendor", "llama.cpp", "build", "bin", llamaServerName),
        ],
    });

    return {
        cliPath,
        agentHostPath: path.join(resources, "bin", agentHostName),
        llamaServerPath: existsSync(llamaServerPath) ? llamaServerPath : undefined,
        webRoot: app.isPackaged ? path.join(resources, "web") : path.join(repoRoot, "web"),
        workspace,
        logDirectory,
        logFile: path.join(logDirectory, "nola-windows.log"),
        licenseDirectory: app.isPackaged
            ? path.join(resources, "licenses")
            : path.join(repoRoot, "windows", "resources", "licenses"),
    };
}

function log(line: string): void {
    const entry = `${new Date().toISOString()} ${line}\n`;
    logStream?.write(entry);
    if (!app.isPackaged) process.stderr.write(entry);
}

function pageHtml(mode: "starting" | "failed", detail = ""): string {
    return startupPageHtml(strings, mode, detail);
}

async function showInternalPage(mode: "starting" | "failed", detail = ""): Promise<void> {
    if (!mainWindow || mainWindow.isDestroyed()) return;
    await mainWindow.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(pageHtml(mode, detail))}`);
}

function allowedExternalUrl(value: string): boolean {
    try {
        const url = new URL(value);
        return ['http:', 'https:', 'mailto:'].includes(url.protocol) && !url.username && !url.password;
    } catch { return false; }
}

function createWindow(): BrowserWindow {
    const savedState = loadWindowState();
    const window = new BrowserWindow({
        width: savedState.width,
        height: savedState.height,
        x: savedState.x,
        y: savedState.y,
        minWidth: Math.min(720, savedState.width),
        minHeight: Math.min(520, savedState.height),
        show: false,
        backgroundColor: nativeTheme.shouldUseDarkColors ? "#000000" : "#ffffff",
        autoHideMenuBar: true,
        icon: path.join(resourcesRoot(), "web", "app", "static", "nola-icon.png"),
        webPreferences: {
            preload: path.join(__dirname, "preload.js"),
            nodeIntegration: false,
            contextIsolation: true,
            sandbox: true,
            webSecurity: true,
            partition: "persist:nola-desktop",
        },
    });
    const syncCanvas = () => { if (!window.isDestroyed()) window.setBackgroundColor(nativeTheme.shouldUseDarkColors ? "#000000" : "#ffffff"); };
    nativeTheme.on("updated", syncCanvas);
    window.once("closed", () => nativeTheme.removeListener("updated", syncCanvas));
    if (savedState.maximized) window.maximize();
    window.once("ready-to-show", () => {
        if (!launchInBackground) window.show();
    });
    window.on("resize", () => scheduleWindowStateSave(window));
    window.on("move", () => scheduleWindowStateSave(window));
    window.on("maximize", () => scheduleWindowStateSave(window));
    window.on("unmaximize", () => scheduleWindowStateSave(window));
    window.on("close", (event) => {
        persistWindowState(window);
        if (quitting) return;
        event.preventDefault();
        window.hide();
        if (!explainedBackgroundMode && process.platform === "win32") {
            explainedBackgroundMode = true;
            tray?.displayBalloon({
                title: strings.backgroundTitle,
                content: strings.backgroundBody,
                iconType: "info",
            });
        }
    });
    window.on("closed", () => { mainWindow = undefined; });
    window.webContents.setWindowOpenHandler(({ url }) => {
        if (isSoftwareUpdateDownload(url, hostOrigin)) {
            window.webContents.downloadURL(url, {headers: {Origin: hostOrigin!}});
            return { action: "deny" };
        }
        if (allowedExternalUrl(url)) void shell.openExternal(url);
        return { action: "deny" };
    });
    window.webContents.on("will-navigate", (event, url) => {
        const isInternal = isTrustedRendererUrl(url);
        if (!isInternal) {
            event.preventDefault();
            if (allowedExternalUrl(url)) void shell.openExternal(url);
        }
    });
    return window;
}

function createMenu(): void {
    const viewItems: Electron.MenuItemConstructorOptions[] = [
        { label: strings.reload, accelerator: "Ctrl+R", click: () => mainWindow?.webContents.reload() },
        { type: "separator" },
        { role: "resetZoom" },
        { role: "zoomIn" },
        { role: "zoomOut" },
        { type: "separator" },
        { role: "togglefullscreen" },
    ];
    if (debugEnabled) {
        viewItems.push({ label: strings.toggleDevTools, accelerator: "Ctrl+Shift+I", click: () => mainWindow?.webContents.toggleDevTools() });
    }
    const menu = Menu.buildFromTemplate([
        {
            label: strings.file,
            submenu: [
                { label: strings.openWorkspace, click: () => void shell.openPath(desktopPaths().workspace) },
                { label: strings.openLogFolder, click: () => void shell.openPath(desktopPaths().logDirectory) },
                { type: "separator" },
                { label: strings.restartEngine, accelerator: "Ctrl+Shift+R", click: () => void restartHost() },
                { type: "separator" },
                { label: strings.quit, accelerator: "Ctrl+Q", click: () => { quitting = true; app.quit(); } },
            ],
        },
        { label: strings.edit, role: "editMenu" },
        {
            label: strings.view,
            submenu: viewItems,
        },
        {
            label: strings.window,
            submenu: [
                { label: strings.minimize, role: "minimize" },
                { label: strings.close, role: "close" },
            ],
        },
        {
            label: strings.help,
            submenu: [
                { label: strings.thirdPartyLicenses, click: () => void shell.openPath(desktopPaths().licenseDirectory) },
            ],
        },
    ]);
    Menu.setApplicationMenu(menu);
}

function createTray(): void {
    const iconPath = path.join(resourcesRoot(), "web", "app", "static", "nola-icon.png");
    const image = nativeImage.createFromPath(iconPath).resize({ width: 20, height: 20 });
    tray = new Tray(image);
    tray.setToolTip("Nola");
    tray.setContextMenu(Menu.buildFromTemplate([
        { label: strings.showNola, click: () => { mainWindow?.show(); mainWindow?.focus(); } },
        { label: strings.restartEngine, click: () => void restartHost() },
        { type: "separator" },
        { label: strings.quit, click: () => { quitting = true; app.quit(); } },
    ]));
    tray.on("double-click", () => { mainWindow?.show(); mainWindow?.focus(); });
}

function configureSession(): void {
    const desktopSession = session.fromPartition("persist:nola-desktop");
    desktopSession.on("will-download", (event, item, contents) => {
        if (!isSoftwareUpdateDownload(item.getURL(), hostOrigin)) return;
        if (!mainWindow || contents.id !== mainWindow.webContents.id) { event.preventDefault(); return; }
        if (item.getMimeType() !== "application/octet-stream" || !/\.(exe|msi)$/i.test(item.getFilename())) {
            event.preventDefault();
            void dialog.showMessageBox(mainWindow, {type: "error", message: strings.updateFailed});
            return;
        }
        item.setSaveDialogOptions({title: strings.updateDownload, defaultPath: path.join(app.getPath("downloads"), item.getFilename())});
        item.once("done", (_event, state) => {
            if (!mainWindow || mainWindow.isDestroyed() || state === "cancelled") return;
            if (state !== "completed") {
                void dialog.showMessageBox(mainWindow, {type: "error", message: strings.updateFailed});
                return;
            }
            void dialog.showMessageBox(mainWindow, {type: "info", message: strings.updateReady,
                buttons: [strings.showDownload, "OK"], defaultId: 0, cancelId: 1}).then(result => {
                    if (result.response === 0) shell.showItemInFolder(item.getSavePath());
                });
        });
    });
    desktopSession.setPermissionRequestHandler((webContents, permission, callback) => {
        const trusted = isTrustedRendererUrl(webContents.getURL());
        callback(trusted && (permission === "notifications" || permission === "clipboard-sanitized-write"));
    });
}

function isTrustedRendererUrl(rawUrl: string): boolean {
    if (rawUrl.startsWith("data:text/html")) return true;
    if (!hostOrigin) return false;
    try {
        return new URL(rawUrl).origin === hostOrigin;
    } catch {
        return false;
    }
}

function handleHostState(state: HostState): void {
    log(`[state] ${JSON.stringify(state)}`);
    if (state.kind === "ready") {
        hostOrigin = state.origin;
        automaticRecoveryCount = 0;
        if (recoveryTimer) clearTimeout(recoveryTimer);
        void mainWindow?.loadURL(`${state.origin}/chat`);
        return;
    }
    if (state.kind === "failed" && !quitting && automaticRecoveryCount < 3) {
        automaticRecoveryCount += 1;
        void showInternalPage("starting");
        recoveryTimer = setTimeout(() => void startHost(), 700 * automaticRecoveryCount);
    }
}

async function startHost(): Promise<void> {
    const paths = desktopPaths();
    if (!host) {
        host = new HostController({
            cliPath: paths.cliPath,
            webRoot: paths.webRoot,
            workspaceRoot: paths.workspace,
            llamaServerPath: paths.llamaServerPath,
            llamaBackend: loadNativeRuntimeInfo().backend,
            debug: debugEnabled,
            log,
            onState: handleHostState,
        });
    }
    await showInternalPage("starting");
    try {
        await host.start();
    } catch (error) {
        if (automaticRecoveryCount >= 3) {
            await showInternalPage("failed", error instanceof Error ? error.message : String(error));
        }
    }
}

async function restartHost(): Promise<void> {
    if (restartingHost || quitting) return;
    restartingHost = true;
    if (recoveryTimer) { clearTimeout(recoveryTimer); recoveryTimer = undefined; }
    automaticRecoveryCount = 0;
    hostOrigin = undefined;
    try {
        await showInternalPage("starting");
        if (host) await host.restart();
        else await startHost();
    } catch (error) {
        await showInternalPage("failed", error instanceof Error ? error.message : String(error));
    } finally { restartingHost = false; }
}

function registerIpc(): void {
    const hasTrustedSender = (event: Electron.IpcMainInvokeEvent): boolean => {
        const frame = event.senderFrame;
        return frame !== null && isTrustedRendererUrl(frame.url);
    };
    ipcMain.handle("desktop:retry-host", (event) => {
        if (!hasTrustedSender(event)) return false;
        void restartHost();
        return true;
    });
    ipcMain.handle("desktop:open-log-folder", (event) => {
        if (!hasTrustedSender(event)) return false;
        void shell.openPath(desktopPaths().logDirectory);
        return true;
    });
    ipcMain.handle("desktop:open-workspace", (event) => {
        if (!hasTrustedSender(event)) return false;
        void shell.openPath(desktopPaths().workspace);
        return true;
    });
    ipcMain.handle("desktop:native-runtime", (event) => {
        if (!hasTrustedSender(event)) return null;
        return loadNativeRuntimeInfo();
    });
    ipcMain.handle("desktop:login-item", (event) => {
        if (!hasTrustedSender(event)) return null;
        const available = process.platform === "win32" && app.isPackaged;
        return {
            available,
            enabled: available && app.getLoginItemSettings().openAtLogin,
            labels: {
                title: strings.startAtLogin,
                body: strings.startAtLoginBody,
                enabled: strings.enabled,
                disabled: strings.disabled,
                unavailable: strings.packagedOnly,
            },
        };
    });
    ipcMain.handle("desktop:set-login-item", (event, requested: unknown) => {
        if (!hasTrustedSender(event) || typeof requested !== "boolean") return null;
        const available = process.platform === "win32" && app.isPackaged;
        if (!available) return { available: false, enabled: false };
        app.setLoginItemSettings({
            openAtLogin: requested,
            path: process.execPath,
            args: ["--background"],
        });
        return { available: true, enabled: app.getLoginItemSettings().openAtLogin };
    });
}

if (!app.requestSingleInstanceLock()) {
    app.quit();
} else {
    app.on("second-instance", () => { mainWindow?.show(); mainWindow?.focus(); });
    app.on("before-quit", () => { quitting = true; });
    app.on("window-all-closed", () => {
        // Keep the local scheduler and Agent Host alive in the notification area.
    });
    app.on("activate", () => { mainWindow?.show(); mainWindow?.focus(); });

    void app.whenReady().then(async () => {
        strings = stringsFor(app.getLocale());
        if (process.platform === "win32") app.setAppUserModelId("com.nola.assistant.windows");
        const paths = desktopPaths();
        logStream = createWriteStream(paths.logFile, { flags: "a" });
        log(`[startup] app=${app.getVersion()} electron=${process.versions.electron} debug=${debugEnabled} background=${launchInBackground} cli=${paths.cliPath}`);
        log(`[startup] web=${paths.webRoot} workspace=${paths.workspace} llama=${paths.llamaServerPath ?? "PATH discovery"}`);
        configureSession();
        registerIpc();
        createMenu();
        mainWindow = createWindow();
        createTray();
        await startHost();
    });

    app.on("will-quit", () => {
        if (recoveryTimer) clearTimeout(recoveryTimer);
        if (windowStateTimer) clearTimeout(windowStateTimer);
        if (mainWindow && !mainWindow.isDestroyed()) persistWindowState(mainWindow);
        void host?.stop();
        logStream?.end();
    });
}
