const invoke = (name, args) => window.__TAURI_INTERNALS__.invoke(name, args);
let phase = "initial";
let latestDesktopError = "";
function renderDesktopError(payload) {
    const message = payload && typeof payload.message === "string" && payload.message.trim()
        ? payload.message.trim()
        : (payload && typeof payload.detail === "string" ? payload.detail.trim() : "");
    if (!message) return;
    latestDesktopError = message;
    const feedback = document.getElementById("nola-action-status");
    if (feedback) {
        feedback.textContent = message;
        return;
    }
    const host = document.querySelector("main") || document.body;
    if (!host) return;
    let status = document.getElementById("nola-tray-status");
    if (!status) {
        status = document.createElement("p");
        status.id = "nola-tray-status";
        status.setAttribute("role", "alert");
        host.append(status);
    }
    status.textContent = message;
}
window.addEventListener("nola:desktop-error", event => {
    renderDesktopError(event && event.detail);
});
async function update() {
    let state;
    try {
        state = await invoke("startup_status", { phase });
    } catch {
        // The webview can start before the command bridge is ready.
        setTimeout(update, 1000);
        return;
    }
    if (!state) return;
    if (state.theme === "dark" || state.theme === "light") {
        document.documentElement.dataset.theme = state.theme;
    } else {
        document.documentElement.dataset.theme = "auto";
    }
    if (state.page) {
        // Both templates come from compiled localization resources. Runtime errors are plain text.
        const doc = new DOMParser().parseFromString(state.page, "text/html");
        document.head.querySelector("style")?.remove();
        document.head.append(doc.querySelector("style"));
        document.querySelector("main").replaceWith(doc.querySelector("main"));
        document.documentElement.lang = state.language;
        document.title = doc.title;
        const detail = document.querySelector(".detail");
        if (detail) detail.textContent = state.failure;
        function bindAction(id, command) {
            const button = document.getElementById(id);
            if (!button) return;
            const feedback = document.getElementById("nola-action-status");
            button.addEventListener("click", async () => {
                if (button.disabled) return;
                const hadFocus = document.activeElement === button;
                button.disabled = true;
                button.setAttribute("aria-busy", "true");
                feedback.textContent = "";
                try {
                    await invoke(command);
                } catch (error) {
                    feedback.textContent = error instanceof Error ? error.message : String(error);
                } finally {
                    button.disabled = false;
                    button.removeAttribute("aria-busy");
                    if (hadFocus && button.isConnected && document.activeElement === document.body) {
                        button.focus({preventScroll: true});
                    }
                }
            });
        }
        bindAction("nola-retry-host", "retry_startup");
        bindAction("nola-open-logs", "open_diagnostics");
        if (latestDesktopError) {
            renderDesktopError({ message: latestDesktopError });
        }
    }
    if (state.trayError) {
        renderDesktopError(state.trayError);
    }
    phase = state.phase;
    if (phase === "starting") setTimeout(update, 400);
}
update();
