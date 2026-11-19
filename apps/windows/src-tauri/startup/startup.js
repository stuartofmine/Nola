const chinese = navigator.language.toLowerCase().startsWith("zh");
const statusText = document.getElementById("status");
const retry = document.getElementById("retry");
if (chinese) {
    document.documentElement.lang = "zh";
    statusText.textContent = "正在打开工作区…";
    retry.textContent = "重新打开";
}
async function update() {
    const failure = await window.__TAURI_INTERNALS__.invoke("startup_failure");
    if (failure) {
        statusText.textContent = failure;
        retry.hidden = false;
    } else {
        setTimeout(update, 400);
    }
}
retry.onclick = () => window.__TAURI_INTERNALS__.invoke("retry_startup");
update();
