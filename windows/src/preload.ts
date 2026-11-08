import { contextBridge, ipcRenderer } from "electron";

document.documentElement.dataset.nolaDesktop = "windows";

contextBridge.exposeInMainWorld("NolaDesktop", Object.freeze({
    platform: "windows",
    retryHost: () => ipcRenderer.invoke("desktop:retry-host"),
    openLogFolder: () => ipcRenderer.invoke("desktop:open-log-folder"),
    openWorkspace: () => ipcRenderer.invoke("desktop:open-workspace"),
}));

window.addEventListener("DOMContentLoaded", () => {
    document.getElementById("nola-retry-host")?.addEventListener("click", () => {
        void ipcRenderer.invoke("desktop:retry-host");
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
    if (location.pathname === "/settings") {
        void ipcRenderer.invoke("desktop:login-item").then((state: {
            available: boolean;
            enabled: boolean;
            labels: { title: string; body: string; enabled: string; disabled: string; unavailable: string };
        } | null) => {
            if (!state) return;
            const container = document.querySelector(".settings, .settings-page, .subpage, main.main-content");
            if (!container || document.getElementById("desktopLoginItemCard")) return;
            const section = document.createElement("section");
            section.id = "desktopLoginItemCard";
            section.className = "section-card desktop-login-card";

            const copy = document.createElement("div");
            const title = document.createElement("strong");
            const body = document.createElement("p");
            title.textContent = state.labels.title;
            body.textContent = state.labels.body;
            copy.append(title, body);

            const button = document.createElement("button");
            button.type = "button";
            button.className = "btn btn-secondary desktop-login-toggle";
            const render = (enabled: boolean, available: boolean) => {
                button.textContent = available
                    ? (enabled ? state.labels.enabled : state.labels.disabled)
                    : state.labels.unavailable;
                button.disabled = !available;
                button.classList.toggle("is-enabled", enabled);
                button.setAttribute("aria-pressed", String(enabled));
            };
            render(state.enabled, state.available);
            button.addEventListener("click", async () => {
                button.disabled = true;
                const next = await ipcRenderer.invoke("desktop:set-login-item", !button.classList.contains("is-enabled")) as { available: boolean; enabled: boolean } | null;
                if (next) render(next.enabled, next.available);
                else render(false, false);
            });
            section.append(copy, button);
            container.appendChild(section);
        });
    }
});
