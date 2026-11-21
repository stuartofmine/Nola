if (window.top === window && location.protocol === "http:" && location.hostname === "127.0.0.1") {
    const invoke = (name, args) => window.__TAURI_INTERNALS__.invoke(name, args);
    Object.defineProperty(window, "NolaDesktop", {value: Object.freeze({
        platform: "windows",
        nativeRuntime: () => invoke("native_runtime"),
        loginItem: () => invoke("login_item"),
        setLoginItem: enabled => invoke("set_login_item", {enabled}),
        setTheme: theme => invoke("set_theme", {theme}),
    })});
    window.addEventListener("DOMContentLoaded", () => {
        document.documentElement.dataset.nolaDesktop = "windows";
    }, {once:true});
}
