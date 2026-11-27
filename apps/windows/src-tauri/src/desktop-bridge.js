if (window.top === window && location.protocol === "http:" && location.hostname === "127.0.0.1") {
    const invoke = (name, args) => window.__TAURI_INTERNALS__.invoke(name, args);
    const currentLocale = () => {
        const docLang = document.documentElement?.lang?.trim();
        if (docLang) return docLang;
        try {
            const saved = localStorage.getItem("nola.web.locale");
            if (saved && saved !== "system") return saved;
        } catch {}
        return null;
    };
    Object.defineProperty(window, "NolaDesktop", {value: Object.freeze({
        platform: "windows",
        nativeRuntime: () => invoke("native_runtime"),
        loginItem: () => invoke("login_item", {locale: currentLocale()}),
        setLoginItem: enabled => invoke("set_login_item", {enabled, locale: currentLocale()}),
        setTheme: theme => invoke("set_theme", {theme}),
        openLogsFolder: () => invoke("open_logs_folder"),
    })});
    const markDesktop = () => {
        document.documentElement.dataset.nolaDesktop = "windows";
    };
    let dismissTimer = null;
    let pendingDesktopError = null;
    const renderDesktopError = payload => {
        const message = payload && typeof payload.message === "string" && payload.message.trim()
            ? payload.message.trim()
            : (payload && typeof payload.detail === "string" ? payload.detail.trim() : "");
        if (!message) return;
        if (!document.body) {
            pendingDesktopError = payload;
            return;
        }
        pendingDesktopError = null;
        if (dismissTimer) {
            clearTimeout(dismissTimer);
            dismissTimer = null;
        }
        const existing = document.getElementById("nola-desktop-error-banner");
        if (existing) existing.remove();
        const isDark = document.documentElement.dataset.theme === "dark"
            || (document.documentElement.dataset.theme !== "light"
                && window.matchMedia?.("(prefers-color-scheme: dark)").matches);
        const banner = document.createElement("div");
        banner.id = "nola-desktop-error-banner";
        banner.setAttribute("role", "alert");
        banner.setAttribute("aria-live", "assertive");
        banner.style.cssText = [
            "position:fixed",
            "right:16px",
            "bottom:16px",
            "z-index:2147483647",
            "max-width:min(420px,calc(100vw - 32px))",
            "display:flex",
            "align-items:flex-start",
            "gap:12px",
            "padding:12px 14px",
            "border-radius:12px",
            "font:13px/1.5 \"Segoe UI Variable\",\"Segoe UI\",\"Microsoft YaHei UI\",sans-serif",
            isDark
                ? "background:#1b1b1e;color:#f5f5f7;border:1px solid rgba(255,255,255,0.14);box-shadow:0 10px 28px rgba(0,0,0,0.45)"
                : "background:#ffffff;color:#111113;border:1px solid #d4d4d8;box-shadow:0 10px 28px rgba(0,0,0,0.14)",
        ].join(";");
        const text = document.createElement("span");
        text.style.cssText = "flex:1;overflow-wrap:anywhere";
        text.textContent = message;
        const close = document.createElement("button");
        close.type = "button";
        close.setAttribute("aria-label", "Dismiss");
        close.textContent = "×";
        close.style.cssText = [
            "flex:none",
            "border:none",
            "background:transparent",
            "color:inherit",
            "font:600 16px/1 inherit",
            "padding:2px 6px",
            "border-radius:6px",
            "cursor:pointer",
            "opacity:0.8",
        ].join(";");
        const dismiss = () => {
            if (dismissTimer) {
                clearTimeout(dismissTimer);
                dismissTimer = null;
            }
            banner.remove();
        };
        close.addEventListener("click", dismiss);
        banner.append(text, close);
        document.body.append(banner);
        dismissTimer = setTimeout(dismiss, 10000);
    };
    window.addEventListener("nola:desktop-error", event => {
        renderDesktopError(event && event.detail);
    });
    if (document.readyState !== "loading") {
        markDesktop();
    } else {
        window.addEventListener("DOMContentLoaded", () => {
            markDesktop();
            if (pendingDesktopError) renderDesktopError(pendingDesktopError);
        }, {once:true});
    }
}
