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
    if (document.readyState !== "loading") {
        markDesktop();
    } else {
        window.addEventListener("DOMContentLoaded", markDesktop, {once:true});
    }
}
