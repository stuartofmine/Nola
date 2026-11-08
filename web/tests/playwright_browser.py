import os
import shutil
from pathlib import Path


def launch_test_chromium(playwright, *, timeout=30_000):
    """Launch the same deterministic Chromium surface for every Web product test."""
    explicit_browser = os.environ.get("NOLA_PLAYWRIGHT_BROWSER", "").strip()
    candidates = [
        explicit_browser,
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        str(
            Path.home()
            / "Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        ),
        shutil.which("google-chrome") or "",
        shutil.which("chromium") or "",
        shutil.which("chromium-browser") or "",
    ]
    browser_path = next(
        (candidate for candidate in candidates if candidate and Path(candidate).is_file()),
        None,
    )
    options = {
        "headless": True,
        "timeout": timeout,
        "args": ["--disable-gpu", "--disable-software-rasterizer"],
    }
    if browser_path:
        options["executable_path"] = browser_path
    return playwright.chromium.launch(**options)
