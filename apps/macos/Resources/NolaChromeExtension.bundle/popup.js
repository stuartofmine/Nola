// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements popup logic in the web client.
// - Interfaces:
//   - statusEl
//   - renderStatus
// - APIs:
//   - none
//
const statusEl = document.getElementById("status");

function renderStatus(prefix, status) {
  if (!status) {
    statusEl.textContent = prefix;
    return;
  }
  statusEl.textContent = `${prefix} ${status.state}: ${status.detail}`;
}

chrome.runtime.sendMessage({ type: "popup_ping" }, (response) => {
  const error = chrome.runtime.lastError;
  if (error) {
    statusEl.textContent = `Connect signal failed: ${error.message}`;
    return;
  }
  renderStatus("Connect signal sent.", response && response.nativeStatus);
});

setTimeout(() => {
  chrome.runtime.sendMessage({ type: "native_status" }, (response) => {
    const error = chrome.runtime.lastError;
    if (error) {
      statusEl.textContent = `Status read failed: ${error.message}`;
      return;
    }
    renderStatus("Native status:", response && response.nativeStatus);
  });
}, 1200);
