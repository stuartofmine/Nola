import os
import shutil
import sys
from pathlib import Path


def launch_test_chromium(playwright, *, timeout=30_000):
    """Launch the same deterministic Chromium surface for every Web product test."""
    explicit_browser = os.environ.get("NOLA_PLAYWRIGHT_BROWSER", "").strip()
    browser_path = explicit_browser or next(
        (found for name in ('google-chrome', 'chromium', 'chromium-browser') if (found := shutil.which(name))),
        None,
    )
    options = {
        "headless": True,
        "timeout": timeout,
        "args": ["--disable-gpu", "--disable-software-rasterizer"],
    }
    if browser_path:
        configured = Path(browser_path).expanduser()
        if not configured.is_absolute():
            configured = Path(__file__).resolve().parents[3] / configured
        options["executable_path"] = str(configured.resolve())
    elif sys.platform in ('darwin', 'win32'):
        options["channel"] = "chrome"
    return playwright.chromium.launch(**options)
