const invoke = (name, args) => window.__TAURI_INTERNALS__.invoke(name, args);
let phase = "initial";
async function update() {
    const state = await invoke("startup_status", { phase });
    if (state.page) {
        // Both templates come from compiled localization resources. Runtime errors are plain text.
        const doc = new DOMParser().parseFromString(state.page, "text/html");
        document.head.querySelector("style")?.remove();
        document.head.append(doc.querySelector("style"));
        document.querySelector("main").replaceWith(doc.querySelector("main"));
        document.documentElement.lang = state.language;
        const detail = document.querySelector(".detail");
        if (detail) detail.textContent = state.failure;
        const retry = document.getElementById("nola-retry-host");
        retry?.addEventListener("click", async () => {
            if (retry.disabled) return;
            retry.disabled = true;
            retry.setAttribute("aria-busy", "true");
            try { await invoke("retry_startup"); }
            finally { retry.disabled = false; retry.removeAttribute("aria-busy"); }
        });
        document.getElementById("nola-open-logs")?.addEventListener("click", () => invoke("open_diagnostics"));
    }
    phase = state.phase;
    if (phase === "starting") setTimeout(update, 400);
}
update();
