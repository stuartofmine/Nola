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
    const isModelsLocation = () =>
        location.pathname === "/models" ||
        (new URLSearchParams(location.search).get("content") ?? "").startsWith("/models");
    let cachedRuntime: Promise<{ architecture?: string; backend?: string } | null> | undefined;
    let badgeObserver: MutationObserver | undefined;
    const attachRuntimeBadge = (runtime: { architecture?: string; backend?: string } | null): boolean => {
        const engineState = document.querySelector("[data-local-runtime-host], .model-engine-state");
        if (!engineState || !runtime?.backend) return false;
        if (engineState.querySelector(".desktop-native-runtime")) return true;
        const badge = document.createElement("span");
        badge.className = "desktop-native-runtime";
        badge.textContent = `${runtime.backend.toUpperCase()} · ${(runtime.architecture ?? "").toUpperCase()}`;
        badge.title = "llama.cpp";
        engineState.appendChild(badge);
        return true;
    };
    const syncNativeRuntimeBadge = () => {
        if (!isModelsLocation()) {
            badgeObserver?.disconnect();
            badgeObserver = undefined;
            return;
        }
        cachedRuntime ??= ipcRenderer.invoke("desktop:native-runtime");
        void cachedRuntime.then((runtime) => {
            if (attachRuntimeBadge(runtime) || !runtime?.backend || !document.body) {
                badgeObserver?.disconnect();
                badgeObserver = undefined;
                return;
            }
            if (!badgeObserver) {
                badgeObserver = new MutationObserver(() => {
                    if (attachRuntimeBadge(runtime) || !isModelsLocation()) {
                        badgeObserver?.disconnect();
                        badgeObserver = undefined;
                    }
                });
                badgeObserver.observe(document.body, { childList: true, subtree: true });
            }
        });
    };
    syncNativeRuntimeBadge();
    window.addEventListener("popstate", syncNativeRuntimeBadge);
    document.addEventListener("click", () => setTimeout(syncNativeRuntimeBadge, 50), true);
});
