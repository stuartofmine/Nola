import { contextBridge, ipcRenderer } from "electron";

document.documentElement.dataset.nolaDesktop = "windows";

contextBridge.exposeInMainWorld("NolaDesktop", Object.freeze({
    platform: "windows",
    retryHost: () => ipcRenderer.invoke("desktop:retry-host"),
    openLogFolder: () => ipcRenderer.invoke("desktop:open-log-folder"),
    openWorkspace: () => ipcRenderer.invoke("desktop:open-workspace"),
    loginItem: () => ipcRenderer.invoke("desktop:login-item"),
    setLoginItem: (enabled: boolean) => ipcRenderer.invoke("desktop:set-login-item", enabled),
}));

window.addEventListener("DOMContentLoaded", () => {
    const retry = document.getElementById("nola-retry-host");
    retry?.addEventListener("click", () => {
        if (!(retry instanceof HTMLButtonElement) || retry.disabled) return;
        retry.disabled = true; retry.setAttribute("aria-busy", "true");
        void ipcRenderer.invoke("desktop:retry-host").then((accepted: boolean) => {
            if (!accepted && retry.isConnected) { retry.disabled = false; retry.removeAttribute("aria-busy"); }
        }).catch(() => {
            if (retry.isConnected) { retry.disabled = false; retry.removeAttribute("aria-busy"); }
        });
    });
    document.getElementById("nola-open-logs")?.addEventListener("click", () => {
        void ipcRenderer.invoke("desktop:open-log-folder");
    });
    if (location.pathname === "/models") {
        void ipcRenderer.invoke("desktop:native-runtime").then((runtime: { architecture?: string; backend?: string } | null) => {
            const engineState = document.querySelector(".model-engine-state");
            if (!engineState || !runtime?.backend) return;
            const badge = document.createElement("span");
            badge.className = "desktop-native-runtime";
            badge.textContent = `${runtime.backend.toUpperCase()} · ${(runtime.architecture ?? "").toUpperCase()}`;
            badge.title = "llama.cpp";
            engineState.appendChild(badge);
        });
    }
});
