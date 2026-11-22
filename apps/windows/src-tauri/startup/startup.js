const invoke = (name, args) => window.__TAURI_INTERNALS__.invoke(name, args);
let phase = "initial";
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
    }
    phase = state.phase;
    if (phase === "starting") setTimeout(update, 400);
}
update();
