import ctypes
import ctypes.util
import hashlib
import json
import os
import platform
import subprocess
import sys
import tempfile
import time
import urllib.parse
from pathlib import Path

MIN_SCREEN_COORDINATE = -100000
MAX_SCREEN_COORDINATE = 100000
MIN_ELEMENT_OFFSET = -10000
MAX_ELEMENT_OFFSET = 10000
DEFAULT_PASTE_SETTLE_MS = 120
CG_MOUSE_EVENT_CLICK_STATE_FIELD = 1
ALLOWED_ELEMENT_ACTIONS = {
    "AXPress",
    "AXShowMenu",
    "AXIncrement",
    "AXDecrement",
    "AXConfirm",
    "AXCancel",
    "AXRaise",
    "AXShowAlternateUI",
    "AXShowDefaultUI",
}
ALLOWED_WINDOW_ACTIONS = {
    "raise",
    "minimize",
    "zoom",
    "enter_full_screen",
    "exit_full_screen",
    "toggle_full_screen",
}
COMPUTER_ACTIONS = [
    "status",
    "observe",
    "list_displays",
    "screenshot",
    "screenshot_element",
    "screenshot_window",
    "read_text",
    "find_text",
    "move_to_text",
    "click_text",
    "select_text",
    "assert_text",
    "wait_for_text",
    "wait",
    "accessibility_snapshot",
    "element_at",
    "press_at",
    "type_at",
    "scroll_at",
    "move_to_element",
    "click_element",
    "press_element",
    "focus_element",
    "perform_element_action",
    "type_element",
    "set_element_value",
    "assert_element",
    "wait_for_element",
    "get_position",
    "move_mouse",
    "mouse_down",
    "mouse_up",
    "click",
    "drag",
    "drag_element",
    "type_text",
    "get_clipboard",
    "set_clipboard",
    "copy_selection",
    "assert_clipboard",
    "wait_for_clipboard",
    "press_key",
    "hotkey",
    "key_down",
    "key_up",
    "scroll",
    "scroll_element",
    "list_apps",
    "assert_app",
    "wait_for_app",
    "list_windows",
    "focus_window",
    "set_window_bounds",
    "perform_window_action",
    "close_window",
    "assert_window",
    "wait_for_window",
    "list_menus",
    "select_menu",
    "activate_app",
    "quit_app",
    "open_item",
]

KEY_CODES = {
    "Enter": 36,
    "Tab": 48,
    "Escape": 53,
    "Backspace": 51,
    "Delete": 117,
    "Space": 49,
    "Home": 115,
    "End": 119,
    "PageUp": 116,
    "PageDown": 121,
    "ArrowLeft": 123,
    "ArrowRight": 124,
    "ArrowDown": 125,
    "ArrowUp": 126,
    "F1": 122,
    "F2": 120,
    "F3": 99,
    "F4": 118,
    "F5": 96,
    "F6": 97,
    "F7": 98,
    "F8": 100,
    "F9": 101,
    "F10": 109,
    "F11": 103,
    "F12": 111,
    "F13": 105,
    "F14": 107,
    "F15": 113,
    "F16": 106,
    "F17": 64,
    "F18": 79,
    "F19": 80,
    "F20": 90,
}
MODIFIERS = {
    "Control": "control down",
    "Meta": "command down",
    "Command": "command down",
    "Shift": "shift down",
    "Alt": "option down",
    "Option": "option down",
}
ALPHANUMERIC_KEY_CODES = {
    "a": 0,
    "s": 1,
    "d": 2,
    "f": 3,
    "h": 4,
    "g": 5,
    "z": 6,
    "x": 7,
    "c": 8,
    "v": 9,
    "b": 11,
    "q": 12,
    "w": 13,
    "e": 14,
    "r": 15,
    "y": 16,
    "t": 17,
    "1": 18,
    "2": 19,
    "3": 20,
    "4": 21,
    "6": 22,
    "5": 23,
    "9": 25,
    "7": 26,
    "8": 28,
    "0": 29,
    "o": 31,
    "u": 32,
    "i": 34,
    "p": 35,
    "l": 37,
    "j": 38,
    "k": 40,
    "n": 45,
    "m": 46,
}
MODIFIER_KEY_CODES = {
    "Command": 55,
    "Shift": 56,
    "Option": 58,
    "Control": 59,
}
KEY_STATE_ALIASES = {
    "Meta": "Command",
    "Alt": "Option",
}


def ok(text, display):
    return {"ok": True, "text": text, "display": display}


def err(text, display="Computer action failed"):
    return {"ok": False, "text": text, "display": display}


def is_macos():
    return sys.platform == "darwin"


def ensure_macos(action):
    if not is_macos():
        raise RuntimeError(f"computer {action} is currently supported on macOS only")


def accessibility_permission(prompt=False):
    if not is_macos():
        return True, "not-macos"
    framework_path = ctypes.util.find_library("ApplicationServices") or (
        "/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices"
    )
    corefoundation_path = ctypes.util.find_library("CoreFoundation") or (
        "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation"
    )
    if not framework_path or not corefoundation_path:
        return False, "application services framework not available"
    try:
        app_services_lib = ctypes.CDLL(framework_path)
        core_foundation = ctypes.CDLL(corefoundation_path)
        app_services_lib.AXIsProcessTrustedWithOptions.argtypes = [ctypes.c_void_p]
        app_services_lib.AXIsProcessTrustedWithOptions.restype = ctypes.c_bool
        core_foundation.CFDictionaryCreateMutable.argtypes = [
            ctypes.c_void_p,
            ctypes.c_long,
            ctypes.c_void_p,
            ctypes.c_void_p,
        ]
        core_foundation.CFDictionaryCreateMutable.restype = ctypes.c_void_p
        core_foundation.CFDictionarySetValue.argtypes = [
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.c_void_p,
        ]
        core_foundation.CFDictionarySetValue.restype = None
        core_foundation.CFRelease.argtypes = [ctypes.c_void_p]
        core_foundation.CFRelease.restype = None

        key_ref = ctypes.c_void_p.in_dll(app_services_lib, "kAXTrustedCheckOptionPrompt")
        true_ref = ctypes.c_void_p.in_dll(core_foundation, "kCFBooleanTrue")
        false_ref = ctypes.c_void_p.in_dll(core_foundation, "kCFBooleanFalse")
        options = core_foundation.CFDictionaryCreateMutable(None, 0, None, None)
        if not options:
            return False, "failed to build AX options dictionary"
        try:
            core_foundation.CFDictionarySetValue(
                options,
                key_ref.value,
                true_ref.value if prompt else false_ref.value,
            )
            trusted = bool(app_services_lib.AXIsProcessTrustedWithOptions(options))
        finally:
            core_foundation.CFRelease(options)
        return trusted, ""
    except Exception as exc:
        return False, f"accessibility check failed: {exc}"


def screen_capture_permission(prompt=False):
    if not is_macos():
        return True, "not-macos"
    framework_path = ctypes.util.find_library("CoreGraphics") or (
        "/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics"
    )
    if not framework_path:
        return False, "core graphics framework not available"
    try:
        core_graphics = ctypes.CDLL(framework_path)
        preflight = getattr(core_graphics, "CGPreflightScreenCaptureAccess", None)
        request = getattr(core_graphics, "CGRequestScreenCaptureAccess", None)
        if preflight is None:
            return False, "CGPreflightScreenCaptureAccess is unavailable"
        preflight.argtypes = []
        preflight.restype = ctypes.c_bool
        if request is not None:
            request.argtypes = []
            request.restype = ctypes.c_bool
        if bool(preflight()):
            return True, ""
        if prompt and request is not None and bool(request()):
            return True, ""
        return False, "screen recording permission is not granted"
    except Exception as exc:
        return False, f"screen capture check failed: {exc}"


def computer_permission_status():
    ax_ok, ax_detail = accessibility_permission(False)
    screen_ok, screen_detail = screen_capture_permission(False)
    return {
        "accessibility_permission": ax_ok,
        "accessibility_detail": ax_detail,
        "screen_recording_permission": screen_ok,
        "screen_recording_detail": screen_detail,
    }


def int_arg(args, key, default, low, high):
    try:
        raw = args.get(key)
        value = int(default if raw is None or raw == "" else raw)
    except (TypeError, ValueError) as exc:
        raise RuntimeError(f"{key} must be an integer") from exc
    if value < low or value > high:
        raise RuntimeError(f"{key} must be between {low} and {high}")
    return value


def float_arg(args, key, low, high):
    try:
        value = float(args.get(key))
    except (TypeError, ValueError) as exc:
        raise RuntimeError(f"{key} must be a number") from exc
    if value < low or value > high:
        raise RuntimeError(f"{key} must be between {low} and {high}") from None
    return value


def optional_int_arg(args, key, low, high):
    if args.get(key) is None or args.get(key) == "":
        return None
    return int_arg(args, key, low, low, high)


def bool_arg(args, key, default):
    raw = args.get(key)
    if raw is None:
        return default
    if isinstance(raw, bool):
        return raw
    if isinstance(raw, str):
        value = raw.strip().lower()
        if value in {"true", "1", "yes", "on"}:
            return True
        if value in {"false", "0", "no", "off"}:
            return False
    if isinstance(raw, (int, float)) and raw in (0, 1):
        return bool(raw)
    return default


def optional_bool_arg(args, key):
    if key not in args or args.get(key) is None or args.get(key) == "":
        return None
    return bool_arg(args, key, False)


def state_expectations(args):
    expectations = {}
    for key in ("enabled", "focused", "selected"):
        expected = optional_bool_arg(args, key)
        if expected is not None:
            expectations[key] = expected
    return expectations


def element_matches_state_expectations(element, expectations):
    for key, expected in expectations.items():
        if element.get(key) is not expected:
            return False
    return True


def quote_applescript(text):
    return str(text or "").replace("\\", "\\\\").replace('"', '\\"')


def applescript_string_expr(text):
    normalized = str(text or "").replace("\r\n", "\n").replace("\r", "\n")
    return " & linefeed & ".join(f'"{quote_applescript(part)}"' for part in normalized.split("\n"))


def target_expression(target):
    value = str(target or "").strip()
    if value.startswith("id:"):
        return f'application id "{quote_applescript(value[3:])}"'
    if value.startswith("app:"):
        return f'application "{quote_applescript(value[4:])}"'
    return f'application "{quote_applescript(value)}"'


def permission_hint_from_osascript_error(detail):
    lower = str(detail or "").lower()
    if not lower:
        return ""
    if (
        "assistive access" in lower
        or "not authorized to send keystrokes" in lower
        or "system events got an error: osascript is not allowed" in lower
    ):
        return (
            "Accessibility permission is required. Grant it to the terminal or Python process "
            "in System Settings > Privacy & Security > Accessibility."
        )
    if "not authorized to send apple events to system events" in lower or "(-1743)" in lower:
        return (
            "Automation permission is required. Allow Apple Events to System Events "
            "in System Settings > Privacy & Security > Automation."
        )
    return ""


def run_osascript(script, timeout=12):
    try:
        completed = subprocess.run(
            ["osascript", "-e", script],
            capture_output=True,
            text=True,
            timeout=max(1, min(float(timeout), 60)),
            check=False,
        )
    except FileNotFoundError as exc:
        raise RuntimeError("osascript is unavailable on this host") from exc
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "").strip()
        hint = permission_hint_from_osascript_error(detail)
        if hint:
            raise RuntimeError(f"{detail} | {hint}")
        raise RuntimeError(detail or f"osascript exited with code {completed.returncode}")
    return (completed.stdout or "").strip()


def paste_text_with_clipboard(text, settle_ms=DEFAULT_PASTE_SETTLE_MS, timeout=8):
    old = subprocess.run(["pbpaste"], capture_output=True, text=True, check=False, timeout=5).stdout
    try:
        subprocess.run(["pbcopy"], input=text, text=True, check=True, timeout=5)
        output = run_osascript('tell application "System Events" to keystroke "v" using command down', timeout=timeout)
        if settle_ms:
            time.sleep(settle_ms / 1000)
        return output
    finally:
        try:
            subprocess.run(["pbcopy"], input=old, text=True, check=False, timeout=5)
        except Exception:
            pass


def clear_focused_text():
    run_osascript(
        'tell application "System Events"\n'
        '    keystroke "a" using command down\n'
        '    key code 51\n'
        'end tell',
        timeout=8,
    )


def press_enter_key():
    run_osascript('tell application "System Events" to key code 36', timeout=8)


def screen_size():
    details = screen_details()
    if not details:
        return ""
    return f"{details['pixel_width']}x{details['pixel_height']}"


def screen_details():
    if not is_macos():
        return {}
    try:
        class LocalCGPoint(ctypes.Structure):
            _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]

        class LocalCGSize(ctypes.Structure):
            _fields_ = [("width", ctypes.c_double), ("height", ctypes.c_double)]

        class LocalCGRect(ctypes.Structure):
            _fields_ = [("origin", LocalCGPoint), ("size", LocalCGSize)]

        cg = app_services()
        cg.CGMainDisplayID.restype = ctypes.c_uint32
        main_display_id = cg.CGMainDisplayID()
        cg.CGDisplayPixelsWide.argtypes = [ctypes.c_uint32]
        cg.CGDisplayPixelsWide.restype = ctypes.c_size_t
        cg.CGDisplayPixelsHigh.argtypes = [ctypes.c_uint32]
        cg.CGDisplayPixelsHigh.restype = ctypes.c_size_t
        cg.CGDisplayBounds.argtypes = [ctypes.c_uint32]
        cg.CGDisplayBounds.restype = LocalCGRect

        display_ids = [int(main_display_id)]
        get_active = getattr(cg, "CGGetActiveDisplayList", None)
        if get_active is not None:
            max_displays = 16
            active = (ctypes.c_uint32 * max_displays)()
            count = ctypes.c_uint32(0)
            get_active.argtypes = [ctypes.c_uint32, ctypes.POINTER(ctypes.c_uint32), ctypes.POINTER(ctypes.c_uint32)]
            get_active.restype = ctypes.c_int32
            if int(get_active(max_displays, active, ctypes.byref(count))) == 0 and count.value:
                display_ids = [int(active[index]) for index in range(min(count.value, max_displays))]

        displays = []
        for display_id in display_ids:
            pixel_width = int(cg.CGDisplayPixelsWide(display_id))
            pixel_height = int(cg.CGDisplayPixelsHigh(display_id))
            bounds = cg.CGDisplayBounds(display_id)
            coordinate_width = float(bounds.size.width)
            coordinate_height = float(bounds.size.height)
            displays.append({
                "display_id": display_id,
                "is_main": display_id == int(main_display_id),
                "pixel_width": pixel_width,
                "pixel_height": pixel_height,
                "coordinate_x": float(bounds.origin.x),
                "coordinate_y": float(bounds.origin.y),
                "coordinate_width": coordinate_width,
                "coordinate_height": coordinate_height,
                "scale_x": pixel_width / coordinate_width if coordinate_width else 1,
                "scale_y": pixel_height / coordinate_height if coordinate_height else 1,
            })
        main = next((item for item in displays if item.get("is_main")), displays[0] if displays else {})
        if not main:
            return {}
        min_x = min(float(item["coordinate_x"]) for item in displays)
        min_y = min(float(item["coordinate_y"]) for item in displays)
        max_x = max(float(item["coordinate_x"]) + float(item["coordinate_width"]) for item in displays)
        max_y = max(float(item["coordinate_y"]) + float(item["coordinate_height"]) for item in displays)
        return {
            "pixel_width": int(main["pixel_width"]),
            "pixel_height": int(main["pixel_height"]),
            "coordinate_x": float(main["coordinate_x"]),
            "coordinate_y": float(main["coordinate_y"]),
            "coordinate_width": float(main["coordinate_width"]),
            "coordinate_height": float(main["coordinate_height"]),
            "scale_x": float(main["scale_x"]),
            "scale_y": float(main["scale_y"]),
            "coordinate_bounds": {
                "x": min_x,
                "y": min_y,
                "width": max_x - min_x,
                "height": max_y - min_y,
            },
            "displays": displays,
        }
    except Exception:
        return {}


def png_dimensions(path):
    try:
        with Path(path).open("rb") as handle:
            header = handle.read(24)
        if header[:8] != b"\x89PNG\r\n\x1a\n" or header[12:16] != b"IHDR":
            return {}
        return {
            "width": int.from_bytes(header[16:20], "big"),
            "height": int.from_bytes(header[20:24], "big"),
        }
    except OSError:
        return {}


def computer_status():
    permissions = computer_permission_status()
    lines = [
        f"Platform: {sys.platform}",
        f"Python: {platform.python_version()}",
        f"Screen size: {screen_size() or '(unknown)'}",
        f"Supported actions: {json.dumps(COMPUTER_ACTIONS, ensure_ascii=False)}",
        f"Accessibility permission: {str(permissions['accessibility_permission']).lower()}",
        f"Screen recording permission: {str(permissions['screen_recording_permission']).lower()}",
    ]
    if permissions.get("accessibility_detail"):
        lines.append(f"Accessibility detail: {permissions['accessibility_detail']}")
    if permissions.get("screen_recording_detail"):
        lines.append(f"Screen recording detail: {permissions['screen_recording_detail']}")
    if is_macos():
        lines.append("Input: macOS Quartz/System Events")
        lines.append("Screenshot: screencapture")
        lines.append("Automation permission: macOS may still require Apple Events approval for System Events.")
    else:
        lines.append("Input: unsupported on this platform")
    return ok("\n".join(lines), "computer status")


def observe(args, state_dir):
    include_screenshot = bool_arg(args, "include_screenshot", bool_arg(args, "screenshot", True))
    include_accessibility = bool_arg(args, "include_accessibility", True)
    include_windows = bool_arg(args, "include_windows", True)
    include_apps = bool_arg(args, "include_apps", False)
    include_position = bool_arg(args, "include_position", True)
    include_values = bool_arg(args, "include_values", True)
    include_actions = bool_arg(args, "include_actions", True)
    include_state = bool_arg(args, "include_state", True)
    include_background_apps = bool_arg(args, "include_background_apps", False)
    include_text = bool_arg(args, "include_text", True)
    include_ocr_text = bool_arg(args, "include_ocr_text", False)
    max_elements = int_arg(args, "max_elements", 80, 1, 500)
    max_windows = int_arg(args, "max_windows", 30, 1, 200)
    max_apps = int_arg(args, "max_apps", 50, 1, 500)
    max_text_results = int_arg(args, "max_text_results", 80, 1, 500)
    text_min_confidence = float(args.get("text_min_confidence", 0.25) if args.get("text_min_confidence") not in (None, "") else 0.25)
    if text_min_confidence < 0 or text_min_confidence > 1:
        raise RuntimeError("text_min_confidence must be between 0 and 1")
    timeout_sec = int_arg(args, "timeout_sec", 20, 1, 120)

    details = {
        "observed_at_ms": int(time.time() * 1000),
        "status": {
            "platform": sys.platform,
            "python": platform.python_version(),
            "screen_size": screen_size() or "",
            "supported_actions": COMPUTER_ACTIONS,
            **computer_permission_status(),
        },
        "include_screenshot": include_screenshot,
        "include_accessibility": include_accessibility,
        "include_windows": include_windows,
        "include_apps": include_apps,
        "include_position": include_position,
        "include_text": include_text,
        "include_ocr_text": include_ocr_text,
    }
    observed = ["status"]
    warnings = []
    visible_text_rows = []
    visible_text_seen = set()
    visible_text_sources = []

    screen = screen_details()
    if screen:
        displays = screen.get("displays") if isinstance(screen.get("displays"), list) else []
        details["screen"] = screen
        details["displays"] = displays
        details["display_count"] = len(displays)
        observed.append("displays")
    else:
        warnings.append("displays failed: screen details are unavailable")

    if include_position:
        try:
            x, y = current_position()
            details["cursor"] = {"x": x, "y": y}
            observed.append("cursor")
        except Exception as exc:
            warnings.append(f"cursor failed: {exc}")

    if include_windows:
        try:
            listed = list_windows({"max_windows": max_windows}, state_dir)
            windows = []
            frontmost = ""
            text = str(listed.get("text") or "")
            for line in text.splitlines():
                if line.startswith("Frontmost app: "):
                    frontmost = line.split(": ", 1)[1].strip()
                if line.startswith("Windows: "):
                    windows = json.loads(line.split(": ", 1)[1])
            details["windows"] = windows
            if frontmost:
                details["frontmost_app"] = frontmost
            observed.append("windows")
        except Exception as exc:
            warnings.append(f"windows failed: {exc}")

    if include_accessibility:
        try:
            meta, elements = accessibility_snapshot_elements({
                "max_elements": max_elements,
                "include_values": include_values,
                "include_actions": include_actions,
                "include_state": include_state,
            })
            details["accessibility"] = {
                "frontmost_app": meta.get("frontmost_app") or "",
                "front_window": meta.get("front_window") or "",
                "elements": elements,
                "max_elements": max_elements,
                "include_values": include_values,
                "include_actions": include_actions,
                "include_state": include_state,
            }
            if meta.get("frontmost_app"):
                details["frontmost_app"] = meta.get("frontmost_app")
            if meta.get("front_window"):
                details["front_window"] = meta.get("front_window")
            observed.append("accessibility")
            if include_text:
                visible_text_sources.append("accessibility")
                ax_rows, visible_text_seen = visible_text_rows_from_accessibility_elements(
                    elements,
                    max_text_results,
                )
                visible_text_rows.extend(ax_rows)
        except Exception as exc:
            warnings.append(f"accessibility failed: {exc}")

    if include_apps:
        try:
            listed = list_apps(
                {"include_background": include_background_apps, "max_apps": max_apps},
                state_dir,
            )
            frontmost, apps = apps_from_list_result(listed)
            details["apps"] = apps
            if frontmost:
                details["frontmost_app"] = frontmost
            observed.append("apps")
        except Exception as exc:
            warnings.append(f"apps failed: {exc}")

    screenshot = None
    if include_screenshot:
        try:
            screenshot_args = {
                key: args.get(key)
                for key in ("x", "y", "width", "height", "display_index", "display_id", "path")
                if args.get(key) is not None and args.get(key) != ""
            }
            screenshot_args["timeout_sec"] = timeout_sec
            screenshot = take_screenshot(screenshot_args, state_dir)
            shot_details = {"path": str(screenshot), "pixel_size": png_dimensions(screenshot)}
            region = screenshot_region(screenshot_args)
            if region:
                shot_details["crop"] = region
            display = screenshot_display_selection(screenshot_args)
            if display:
                shot_details["display"] = display
            details["screenshot"] = shot_details
            observed.append("screenshot")
        except Exception as exc:
            warnings.append(f"screenshot failed: {exc}")

    if include_text and include_ocr_text and len(visible_text_rows) < max_text_results:
        visible_text_sources.append("ocr")
        ocr_screenshot = screenshot
        if ocr_screenshot is None:
            screenshot_args = {
                key: args.get(key)
                for key in ("x", "y", "width", "height", "display_index", "display_id", "path")
                if args.get(key) is not None and args.get(key) != ""
            }
            screenshot_args["timeout_sec"] = timeout_sec
            try:
                ocr_screenshot = take_screenshot(screenshot_args, state_dir)
            except Exception as exc:
                warnings.append(f"ocr text screenshot failed: {exc}")
                ocr_screenshot = None
        if ocr_screenshot is not None:
            try:
                region = ocr_source_region(args, ocr_screenshot)
                for row in run_vision_text_recognition(ocr_screenshot, timeout_sec):
                    text_value = str(row.get("text") or "").strip()
                    if not text_value:
                        continue
                    confidence = float(row.get("confidence") or 0)
                    if confidence < text_min_confidence:
                        continue
                    nx = float(row.get("x") or 0)
                    ny = float(row.get("y") or 0)
                    nw = float(row.get("width") or 0)
                    nh = float(row.get("height") or 0)
                    x = region["x"] + nx * region["width"]
                    y = region["y"] + (1.0 - ny - nh) * region["height"]
                    width = nw * region["width"]
                    height = nh * region["height"]
                    append_visible_text_row(
                        visible_text_rows,
                        visible_text_seen,
                        {
                            "source": "ocr",
                            "text": text_value,
                            "confidence": confidence,
                            "x": x,
                            "y": y,
                            "width": width,
                            "height": height,
                            "center_x": x + width / 2,
                            "center_y": y + height / 2,
                        },
                        max_text_results,
                    )
                    if len(visible_text_rows) >= max_text_results:
                        break
            except Exception as exc:
                warnings.append(f"ocr text failed: {exc}")

    if include_text:
        details["visible_text"] = {
            "texts": visible_text_rows[:max_text_results],
            "text_count": len(visible_text_rows[:max_text_results]),
            "full_text": "\n".join(str(item.get("text") or "") for item in visible_text_rows[:max_text_results]),
            "sources": visible_text_sources,
            "max_text_results": max_text_results,
            "include_ocr_text": include_ocr_text,
        }
        if visible_text_rows:
            observed.append("text")

    details["observed_components"] = observed
    details["warnings"] = warnings
    output_lines = []
    if include_text:
        output_lines.append(f"Visible text: {json.dumps(visible_text_rows[:max_text_results], ensure_ascii=False)}")
    output_lines.append(f"Observation: {json.dumps(details, ensure_ascii=False)}")
    text = (
        "Computer action: observe\n"
        f"Observed components: {', '.join(observed)}\n"
        + "\n".join(output_lines)
    )
    if screenshot is not None:
        text += f"\nScreenshot: {screenshot}"
    if warnings:
        text += f"\nWarnings: {json.dumps(warnings, ensure_ascii=False)}"
    result = ok(text, f"observed desktop components: {', '.join(observed)}")
    result["details"] = details
    if screenshot is not None:
        result["screenshot"] = str(screenshot)
    return result


def list_displays(args, state_dir):
    ensure_macos("list displays")
    details = screen_details()
    if not details:
        raise RuntimeError("screen details are unavailable")
    displays = details.get("displays") if isinstance(details.get("displays"), list) else []
    main_display = next((item for item in displays if item.get("is_main")), displays[0] if displays else {})
    summary = {
        "display_count": len(displays),
        "main_display": main_display,
        "coordinate_bounds": details.get("coordinate_bounds") or {},
        "displays": displays,
        "screen": details,
    }
    text = (
        "Computer action: list_displays"
        f"\nScreen size: {screen_size() or '(unknown)'}"
        f"\nDisplay count: {len(displays)}"
        f"\nDisplay details: {json.dumps(summary, ensure_ascii=False)}"
    )
    return ok(text, f"listed {len(displays)} displays")


def screenshot_path(raw_path, state_dir):
    value = str(raw_path or "").strip()
    if value:
        path = Path(value).expanduser()
        if path.suffix.lower() != ".png":
            path = path.with_suffix(".png")
        path.parent.mkdir(parents=True, exist_ok=True)
        return path
    root = Path(state_dir or tempfile.gettempdir()) / "computer"
    root.mkdir(parents=True, exist_ok=True)
    return root / f"computer-screenshot-{int(time.time() * 1000)}.png"


def crop_int(args, key, low, high):
    try:
        value = int(float(args.get(key)))
    except (TypeError, ValueError) as exc:
        raise RuntimeError(f"{key} must be a number") from exc
    if value < low or value > high:
        raise RuntimeError(f"{key} must be between {low} and {high}")
    return value


def screenshot_region(args):
    keys = ("x", "y", "width", "height")
    has_any = any(args.get(key) is not None and args.get(key) != "" for key in keys)
    has_display_index = args.get("display_index") is not None and args.get("display_index") != ""
    has_display_id = args.get("display_id") is not None and args.get("display_id") != ""
    if has_display_index or has_display_id:
        if has_any:
            raise RuntimeError("display_index/display_id cannot be combined with x/y/width/height")
        region, _display = screenshot_display_region(args)
        return region
    if not has_any:
        return None
    if any(args.get(key) is None or args.get(key) == "" for key in keys):
        raise RuntimeError("x, y, width, and height must be provided together")
    return {
        "x": crop_int(args, "x", MIN_SCREEN_COORDINATE, MAX_SCREEN_COORDINATE),
        "y": crop_int(args, "y", MIN_SCREEN_COORDINATE, MAX_SCREEN_COORDINATE),
        "width": crop_int(args, "width", 1, 100000),
        "height": crop_int(args, "height", 1, 100000),
    }


def screenshot_display_region(args):
    has_display_index = args.get("display_index") is not None and args.get("display_index") != ""
    has_display_id = args.get("display_id") is not None and args.get("display_id") != ""
    if has_display_index and has_display_id:
        raise RuntimeError("provide display_index or display_id, not both")
    details = screen_details()
    displays = details.get("displays") if isinstance(details.get("displays"), list) else []
    if not displays:
        raise RuntimeError("display details are unavailable")
    if has_display_index:
        index = int_arg(args, "display_index", 1, 1, len(displays))
        display = displays[index - 1]
    else:
        display_id = int_arg(args, "display_id", 1, 1, 2147483647)
        display = next((item for item in displays if int(item.get("display_id") or 0) == display_id), None)
        if display is None:
            raise RuntimeError(f"display_id {display_id} was not found")
    region = {
        "x": int(round(float(display.get("coordinate_x") or 0))),
        "y": int(round(float(display.get("coordinate_y") or 0))),
        "width": int(round(float(display.get("coordinate_width") or 0))),
        "height": int(round(float(display.get("coordinate_height") or 0))),
    }
    if region["width"] <= 0 or region["height"] <= 0:
        raise RuntimeError("selected display has invalid bounds")
    return region, display


def screenshot_display_selection(args):
    has_display_index = args.get("display_index") is not None and args.get("display_index") != ""
    has_display_id = args.get("display_id") is not None and args.get("display_id") != ""
    if not has_display_index and not has_display_id:
        return None
    _region, display = screenshot_display_region(args)
    return display


def take_screenshot(args, state_dir):
    ensure_macos("screenshot")
    path = screenshot_path(args.get("path"), state_dir)
    region = screenshot_region(args)
    command = ["screencapture", "-x"]
    if region:
        command.extend(["-R", f"{region['x']},{region['y']},{region['width']},{region['height']}"])
    command.append(str(path))
    completed = subprocess.run(
        command,
        capture_output=True,
        text=True,
        timeout=int(args.get("timeout_sec") or 20),
        check=False,
    )
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "screencapture failed").strip()
        if not detail or "permission" in detail.lower() or "not authorized" in detail.lower() or "denied" in detail.lower():
            detail = (
                f"{detail} | " if detail else ""
            ) + "Screen Recording permission is required for screenshots and OCR. Grant it to the terminal or Python process in System Settings > Privacy & Security > Screen Recording."
        raise RuntimeError(detail)
    return path


def screenshot_result(args, state_dir, action="screenshot"):
    path = take_screenshot(args, state_dir)
    selected_display = screenshot_display_selection(args)
    lines = [
        f"Computer action: {action}",
        f"Platform: {sys.platform}",
        f"Screen size: {screen_size() or '(unknown)'}",
        f"Screenshot: {path}",
        f"Screenshot pixels: {json.dumps(png_dimensions(path), ensure_ascii=False)}",
        f"Screen details: {json.dumps(screen_details(), ensure_ascii=False)}",
    ]
    region = screenshot_region(args)
    if region:
        lines.append(f"Crop: {region['x']},{region['y']},{region['width']},{region['height']}")
    if selected_display:
        lines.append(f"Display: {json.dumps(selected_display, ensure_ascii=False)}")
    return ok("\n".join(lines), f"{action}, screenshot saved")


def text_search_query(args):
    query = str(args.get("text") or "").strip()
    if not query:
        raise RuntimeError("text is required")
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("text is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    return query, match_mode, bool_arg(args, "case_sensitive", False)


def text_search_matches(value, query, match_mode, case_sensitive=False):
    text = str(value or "").strip()
    if not text:
        return False
    expected = query if case_sensitive else query.lower()
    candidate = text if case_sensitive else text.lower()
    if match_mode == "exact":
        return candidate == expected
    return expected in candidate


def ocr_source_region(args, screenshot_path):
    region = screenshot_region(args)
    if region:
        return {key: float(value) for key, value in region.items()}
    display = screenshot_display_selection(args)
    if display:
        return {
            "x": float(display.get("coordinate_x") or 0),
            "y": float(display.get("coordinate_y") or 0),
            "width": float(display.get("coordinate_width") or 0),
            "height": float(display.get("coordinate_height") or 0),
        }
    details = screen_details()
    bounds = details.get("coordinate_bounds") if isinstance(details.get("coordinate_bounds"), dict) else None
    if bounds:
        return {
            "x": float(bounds.get("x") or 0),
            "y": float(bounds.get("y") or 0),
            "width": float(bounds.get("width") or 0),
            "height": float(bounds.get("height") or 0),
        }
    return {
        "x": float(details.get("coordinate_x") or 0),
        "y": float(details.get("coordinate_y") or 0),
        "width": float(details.get("coordinate_width") or 0),
        "height": float(details.get("coordinate_height") or 0),
    }


def run_vision_text_recognition(path, timeout_sec):
    script = f'''
import Foundation
import ImageIO
import Vision

let url = URL(fileURLWithPath: {json.dumps(str(path))})
guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else {{
    throw NSError(domain: "NolaOCR", code: 1, userInfo: [NSLocalizedDescriptionKey: "failed to load screenshot image"])
}}
let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.usesLanguageCorrection = true
let handler = VNImageRequestHandler(cgImage: image, options: [:])
try handler.perform([request])
let rows: [[String: Any]] = (request.results ?? []).compactMap {{ observation in
    guard let candidate = observation.topCandidates(1).first else {{ return nil }}
    let box = observation.boundingBox
    return [
        "text": candidate.string,
        "confidence": candidate.confidence,
        "x": box.origin.x,
        "y": box.origin.y,
        "width": box.size.width,
        "height": box.size.height,
    ]
}}
let data = try JSONSerialization.data(withJSONObject: rows, options: [])
FileHandle.standardOutput.write(data)
'''
    env = os.environ.copy()
    env.setdefault("CLANG_MODULE_CACHE_PATH", str(Path(tempfile.gettempdir()) / "nola-swift-module-cache"))
    completed = subprocess.run(
        ["swift", "-e", script],
        capture_output=True,
        text=True,
        timeout=timeout_sec,
        check=False,
        env=env,
    )
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "Vision text recognition failed").strip()
        raise RuntimeError(detail.splitlines()[0] if detail else "Vision text recognition failed")
    rows = json.loads(completed.stdout or "[]")
    if not isinstance(rows, list):
        raise RuntimeError("Vision text recognition returned invalid data")
    return [row for row in rows if isinstance(row, dict)]


def find_text(args, state_dir):
    query, match_mode, case_sensitive = text_search_query(args)
    max_results = int_arg(args, "max_results", 20, 1, 100)
    max_elements = int_arg(args, "max_elements", 160, 1, 500)
    raw_confidence = args.get("min_confidence", 0.25)
    min_confidence = float(0.25 if raw_confidence is None or raw_confidence == "" else raw_confidence)
    if min_confidence < 0 or min_confidence > 1:
        raise RuntimeError("min_confidence must be between 0 and 1")
    include_accessibility = bool_arg(args, "include_accessibility", True)
    include_ocr = bool_arg(args, "include_ocr", True)
    if not include_accessibility and not include_ocr:
        raise RuntimeError("include_accessibility or include_ocr must be enabled")

    matches = []
    candidates = []
    warnings = []
    screenshot = None

    if include_accessibility:
        try:
            snapshot = accessibility_snapshot({"max_elements": max_elements, "include_values": True, "include_state": True})
            for element in snapshot.get("details", {}).get("elements", []):
                text_parts = [
                    str(element.get(field) or "").strip()
                    for field in ("title", "description", "value")
                    if str(element.get(field) or "").strip()
                ]
                candidates.extend(text_parts[:2])
                matched = next(
                    (
                        text
                        for text in text_parts
                        if text_search_matches(text, query, match_mode, case_sensitive)
                    ),
                    "",
                )
                if not matched:
                    continue
                item = {
                    "source": "accessibility",
                    "text": matched,
                    "index": element.get("index"),
                    "role": element.get("role"),
                }
                bounds = element_bounds(element)
                if bounds:
                    x, y, width, height = bounds
                    item.update({
                        "x": x,
                        "y": y,
                        "width": width,
                        "height": height,
                        "center_x": x + width / 2,
                        "center_y": y + height / 2,
                    })
                matches.append({key: value for key, value in item.items() if value not in ("", None)})
                if len(matches) >= max_results:
                    break
        except Exception as exc:
            warnings.append(f"accessibility search failed: {exc}")

    if include_ocr and len(matches) < max_results:
        screenshot_args = {
            key: args.get(key)
            for key in ("x", "y", "width", "height", "display_index", "display_id", "path", "timeout_sec")
            if args.get(key) is not None and args.get(key) != ""
        }
        try:
            screenshot = take_screenshot(screenshot_args, state_dir)
            region = ocr_source_region(screenshot_args, screenshot)
            for row in run_vision_text_recognition(screenshot, int_arg(args, "timeout_sec", 20, 1, 120)):
                text = str(row.get("text") or "").strip()
                if text:
                    candidates.append(text)
                if not text or not text_search_matches(text, query, match_mode, case_sensitive):
                    continue
                confidence = float(row.get("confidence") or 0)
                if confidence < min_confidence:
                    continue
                nx = float(row.get("x") or 0)
                ny = float(row.get("y") or 0)
                nw = float(row.get("width") or 0)
                nh = float(row.get("height") or 0)
                x = region["x"] + nx * region["width"]
                y = region["y"] + (1.0 - ny - nh) * region["height"]
                width = nw * region["width"]
                height = nh * region["height"]
                matches.append({
                    "source": "ocr",
                    "text": text,
                    "confidence": confidence,
                    "x": x,
                    "y": y,
                    "width": width,
                    "height": height,
                    "center_x": x + width / 2,
                    "center_y": y + height / 2,
                })
                if len(matches) >= max_results:
                    break
        except Exception as exc:
            warnings.append(f"ocr search failed: {exc}")

    details = {
        "text": query,
        "match_mode": match_mode,
        "case_sensitive": case_sensitive,
        "matches": matches[:max_results],
        "match_count": len(matches[:max_results]),
        "candidate_texts": candidates[:20],
        "warnings": warnings,
    }
    text = (
        f"Computer action: find_text\n"
        f"Query: {query}\n"
        f"Matches: {json.dumps(matches[:max_results], ensure_ascii=False)}\n"
        f"Candidate text: {json.dumps(candidates[:20], ensure_ascii=False)}"
    )
    if warnings:
        text += f"\nWarnings: {json.dumps(warnings, ensure_ascii=False)}"
    if screenshot is not None and args.get("screenshot", False):
        text += f"\nScreenshot: {screenshot}"
    if not matches:
        outcome = err(text, f'no text matched "{query}"')
    else:
        outcome = ok(text, f"found {len(matches[:max_results])} text match(es)")
    outcome["details"] = details
    if screenshot is not None:
        outcome["screenshot"] = str(screenshot)
    return outcome


def visible_text_parts_from_element(element):
    parts = []
    seen = set()
    for field in ("title", "description", "value"):
        text = str(element.get(field) or "").strip()
        if not text:
            continue
        key = text.casefold()
        if key in seen:
            continue
        seen.add(key)
        parts.append(text)
    return parts


def append_visible_text_row(rows, seen, item, max_results):
    text = str(item.get("text") or "").strip()
    if not text or len(rows) >= max_results:
        return
    try:
        x_key = int(round(float(item.get("x") or 0)))
        y_key = int(round(float(item.get("y") or 0)))
    except (TypeError, ValueError):
        x_key = 0
        y_key = 0
    key = (str(item.get("source") or ""), text.casefold(), x_key, y_key)
    if key in seen:
        return
    seen.add(key)
    rows.append({key: value for key, value in item.items() if value not in ("", None)})


def visible_text_rows_from_accessibility_elements(elements, max_results):
    rows = []
    seen = set()
    for element in elements:
        if not isinstance(element, dict):
            continue
        text_parts = visible_text_parts_from_element(element)
        if not text_parts:
            continue
        item = {
            "source": "accessibility",
            "text": " | ".join(text_parts),
            "index": element.get("index"),
            "role": element.get("role"),
            "title": element.get("title"),
            "description": element.get("description"),
            "value": element.get("value"),
        }
        bounds = element_bounds(element)
        if bounds:
            x, y, width, height = bounds
            item.update({
                "x": x,
                "y": y,
                "width": width,
                "height": height,
                "center_x": x + width / 2,
                "center_y": y + height / 2,
            })
        append_visible_text_row(rows, seen, item, max_results)
        if len(rows) >= max_results:
            break
    return rows, seen


def read_text(args, state_dir):
    max_results = int_arg(args, "max_results", 120, 1, 500)
    max_elements = int_arg(args, "max_elements", 200, 1, 500)
    raw_confidence = args.get("min_confidence", 0.25)
    min_confidence = float(0.25 if raw_confidence is None or raw_confidence == "" else raw_confidence)
    if min_confidence < 0 or min_confidence > 1:
        raise RuntimeError("min_confidence must be between 0 and 1")
    include_accessibility = bool_arg(args, "include_accessibility", True)
    include_ocr = bool_arg(args, "include_ocr", True)
    if not include_accessibility and not include_ocr:
        raise RuntimeError("include_accessibility or include_ocr must be enabled")

    rows = []
    seen = set()
    warnings = []
    sources = []
    frontmost_app = ""
    front_window = ""
    screenshot = None

    if include_accessibility:
        sources.append("accessibility")
        try:
            snapshot = accessibility_snapshot({"max_elements": max_elements, "include_values": True, "include_state": True})
            details = snapshot.get("details", {})
            frontmost_app = str(details.get("frontmost_app") or "")
            front_window = str(details.get("front_window") or "")
            rows, seen = visible_text_rows_from_accessibility_elements(
                details.get("elements", []),
                max_results,
            )
        except Exception as exc:
            warnings.append(f"accessibility read failed: {exc}")

    if include_ocr and len(rows) < max_results:
        sources.append("ocr")
        screenshot_args = {
            key: args.get(key)
            for key in ("x", "y", "width", "height", "display_index", "display_id", "path", "timeout_sec")
            if args.get(key) is not None and args.get(key) != ""
        }
        try:
            screenshot = take_screenshot(screenshot_args, state_dir)
            region = ocr_source_region(screenshot_args, screenshot)
            for row in run_vision_text_recognition(screenshot, int_arg(args, "timeout_sec", 20, 1, 120)):
                text = str(row.get("text") or "").strip()
                if not text:
                    continue
                confidence = float(row.get("confidence") or 0)
                if confidence < min_confidence:
                    continue
                nx = float(row.get("x") or 0)
                ny = float(row.get("y") or 0)
                nw = float(row.get("width") or 0)
                nh = float(row.get("height") or 0)
                x = region["x"] + nx * region["width"]
                y = region["y"] + (1.0 - ny - nh) * region["height"]
                width = nw * region["width"]
                height = nh * region["height"]
                append_visible_text_row(
                    rows,
                    seen,
                    {
                        "source": "ocr",
                        "text": text,
                        "confidence": confidence,
                        "x": x,
                        "y": y,
                        "width": width,
                        "height": height,
                        "center_x": x + width / 2,
                        "center_y": y + height / 2,
                    },
                    max_results,
                )
                if len(rows) >= max_results:
                    break
        except Exception as exc:
            warnings.append(f"ocr read failed: {exc}")

    details = {
        "texts": rows[:max_results],
        "text_count": len(rows[:max_results]),
        "full_text": "\n".join(str(item.get("text") or "") for item in rows[:max_results]),
        "sources": sources,
        "frontmost_app": frontmost_app,
        "front_window": front_window,
        "warnings": warnings,
    }
    text = (
        "Computer action: read_text\n"
        f"Text count: {len(rows[:max_results])}\n"
        f"Visible text: {json.dumps(rows[:max_results], ensure_ascii=False)}"
    )
    if warnings:
        text += f"\nWarnings: {json.dumps(warnings, ensure_ascii=False)}"
    if screenshot is not None and bool_arg(args, "screenshot", False):
        text += f"\nScreenshot: {screenshot}"
    if not rows and warnings and len(warnings) >= len(sources):
        outcome = err(text, "read text failed")
    else:
        outcome = ok(text, f"read {len(rows[:max_results])} visible text item(s)")
    outcome["details"] = details
    if screenshot is not None:
        outcome["screenshot"] = str(screenshot)
    return outcome


def click_text(args, state_dir):
    ensure_macos("click text")
    source = str(args.get("source") or "any").strip().lower()
    if source not in {"any", "accessibility", "ocr"}:
        raise RuntimeError("source must be any/accessibility/ocr")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    offset_x = element_offset(args, "offset_x")
    offset_y = element_offset(args, "offset_y")
    find_args = dict(args)
    find_args["max_results"] = max(occurrence, int_arg(args, "max_results", 20, 1, 100))
    find_args["screenshot"] = False
    found = find_text(find_args, state_dir)
    if not found.get("ok"):
        return found
    details = found.get("details") if isinstance(found.get("details"), dict) else {}
    raw_matches = details.get("matches") if isinstance(details.get("matches"), list) else []
    matches = [
        match
        for match in raw_matches
        if isinstance(match, dict) and (source == "any" or str(match.get("source") or "").lower() == source)
    ]
    if len(matches) < occurrence:
        result = err(
            "Computer action: click_text\n"
            f"Query: {details.get('text') or args.get('text')}\n"
            f"Occurrence: {occurrence}\n"
            f"Source: {source}\n"
            f"Matches: {json.dumps(matches, ensure_ascii=False)}",
            f"text match occurrence {occurrence} was not found",
        )
        result["details"] = {**details, "source": source, "occurrence": occurrence, "filtered_match_count": len(matches)}
        return result
    target = matches[occurrence - 1]
    try:
        x = float(target.get("center_x")) + offset_x
        y = float(target.get("center_y")) + offset_y
    except (TypeError, ValueError) as exc:
        raise RuntimeError("matched text has no usable click point") from exc
    click_args = {
        key: args.get(key)
        for key in ("button", "clicks", "modifiers", "wait_after_ms", "screenshot")
        if args.get(key) is not None
    }
    click_args["x"] = x
    click_args["y"] = y
    clicked = click(click_args, state_dir)
    text = (
        "Computer action: click_text\n"
        f"Query: {details.get('text') or args.get('text')}\n"
        f"Clicked: {x:g},{y:g}\n"
        f"Target: {json.dumps(target, ensure_ascii=False)}"
    )
    if clicked.get("text"):
        text += "\n" + "\n".join(str(clicked.get("text") or "").splitlines()[1:])
    clicked["text"] = text
    clicked["display"] = f"clicked text {target.get('text') or details.get('text')}"
    clicked["details"] = {
        "target_text": target,
        "text": details.get("text"),
        "source": source,
        "occurrence": occurrence,
        "x": x,
        "y": y,
        "offset_x": offset_x,
        "offset_y": offset_y,
        "match_count": details.get("match_count") or 0,
        "warnings": details.get("warnings") or [],
    }
    return clicked


def select_text(args, state_dir):
    ensure_macos("select text")
    source = str(args.get("source") or "any").strip().lower()
    if source not in {"any", "accessibility", "ocr"}:
        raise RuntimeError("source must be any/accessibility/ocr")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    start_offset_x = element_offset_default(args, "start_offset_x", 2.0)
    end_offset_x = element_offset_default(args, "end_offset_x", -2.0)
    offset_y = element_offset(args, "offset_y")
    duration_ms = int_arg(args, "duration_ms", 350, 0, 10000)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    find_args = dict(args)
    find_args["max_results"] = max(occurrence, int_arg(args, "max_results", 20, 1, 100))
    find_args["screenshot"] = False
    found = find_text(find_args, state_dir)
    if not found.get("ok"):
        return found
    details = found.get("details") if isinstance(found.get("details"), dict) else {}
    raw_matches = details.get("matches") if isinstance(details.get("matches"), list) else []
    matches = [
        match
        for match in raw_matches
        if isinstance(match, dict) and (source == "any" or str(match.get("source") or "").lower() == source)
    ]
    if len(matches) < occurrence:
        result = err(
            "Computer action: select_text\n"
            f"Query: {details.get('text') or args.get('text')}\n"
            f"Occurrence: {occurrence}\n"
            f"Source: {source}\n"
            f"Matches: {json.dumps(matches, ensure_ascii=False)}",
            f"text match occurrence {occurrence} was not found",
        )
        result["details"] = {**details, "source": source, "occurrence": occurrence, "filtered_match_count": len(matches)}
        return result
    target = matches[occurrence - 1]
    try:
        x = float(target.get("x"))
        y = float(target.get("y"))
        width = float(target.get("width"))
        height = float(target.get("height"))
    except (TypeError, ValueError) as exc:
        raise RuntimeError("matched text has no usable selection bounds") from exc
    if width <= 0 or height <= 0:
        raise RuntimeError("matched text has invalid selection bounds")
    start_x = x + start_offset_x
    end_x = x + width + end_offset_x
    if abs(end_x - start_x) < 1:
        start_x = x
        end_x = x + width
    if start_x > end_x:
        start_x, end_x = end_x, start_x
    line_y = y + height / 2 + offset_y
    drag_args = {
        "start_x": start_x,
        "start_y": line_y,
        "end_x": end_x,
        "end_y": line_y,
        "duration_ms": duration_ms,
        "wait_after_ms": wait_after_ms,
        "modifiers": args.get("modifiers"),
        "screenshot": args.get("screenshot", True),
    }
    selected = drag(drag_args, state_dir)
    if not selected.get("ok"):
        selected["details"] = {
            **(selected.get("details") if isinstance(selected.get("details"), dict) else {}),
            "target_text": target,
            "source": source,
            "occurrence": occurrence,
        }
        return selected
    text = (
        "Computer action: select_text\n"
        f"Query: {details.get('text') or args.get('text')}\n"
        f"From: {start_x:g},{line_y:g}\n"
        f"To: {end_x:g},{line_y:g}\n"
        f"Target: {json.dumps(target, ensure_ascii=False)}"
    )
    if selected.get("text"):
        text += "\n" + "\n".join(str(selected.get("text") or "").splitlines()[1:])
    selected["text"] = text
    selected["display"] = f"selected text {target.get('text') or details.get('text')}"
    selected["details"] = {
        "target_text": target,
        "text": details.get("text"),
        "source": source,
        "occurrence": occurrence,
        "selection_start": {"x": start_x, "y": line_y},
        "selection_end": {"x": end_x, "y": line_y},
        "start_offset_x": start_offset_x,
        "end_offset_x": end_offset_x,
        "offset_y": offset_y,
        "duration_ms": duration_ms,
        "wait_after_ms": wait_after_ms,
        "match_count": details.get("match_count") or 0,
        "warnings": details.get("warnings") or [],
    }
    return selected


def move_to_text(args, state_dir):
    ensure_macos("move to text")
    source = str(args.get("source") or "any").strip().lower()
    if source not in {"any", "accessibility", "ocr"}:
        raise RuntimeError("source must be any/accessibility/ocr")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    offset_x = element_offset(args, "offset_x")
    offset_y = element_offset(args, "offset_y")
    find_args = dict(args)
    find_args["max_results"] = max(occurrence, int_arg(args, "max_results", 20, 1, 100))
    find_args["screenshot"] = False
    found = find_text(find_args, state_dir)
    if not found.get("ok"):
        return found
    details = found.get("details") if isinstance(found.get("details"), dict) else {}
    raw_matches = details.get("matches") if isinstance(details.get("matches"), list) else []
    matches = [
        match
        for match in raw_matches
        if isinstance(match, dict) and (source == "any" or str(match.get("source") or "").lower() == source)
    ]
    if len(matches) < occurrence:
        result = err(
            "Computer action: move_to_text\n"
            f"Query: {details.get('text') or args.get('text')}\n"
            f"Occurrence: {occurrence}\n"
            f"Source: {source}\n"
            f"Matches: {json.dumps(matches, ensure_ascii=False)}",
            f"text match occurrence {occurrence} was not found",
        )
        result["details"] = {**details, "source": source, "occurrence": occurrence, "filtered_match_count": len(matches)}
        return result
    target = matches[occurrence - 1]
    try:
        x = float(target.get("center_x")) + offset_x
        y = float(target.get("center_y")) + offset_y
    except (TypeError, ValueError) as exc:
        raise RuntimeError("matched text has no usable move point") from exc
    move_args = {
        key: args.get(key)
        for key in ("wait_after_ms", "screenshot")
        if args.get(key) is not None
    }
    move_args["x"] = x
    move_args["y"] = y
    moved = move_mouse(move_args, state_dir)
    text = (
        "Computer action: move_to_text\n"
        f"Query: {details.get('text') or args.get('text')}\n"
        f"Moved: {x:g},{y:g}\n"
        f"Target: {json.dumps(target, ensure_ascii=False)}"
    )
    if moved.get("text"):
        text += "\n" + "\n".join(str(moved.get("text") or "").splitlines()[1:])
    moved["text"] = text
    moved["display"] = f"moved to text {target.get('text') or details.get('text')}"
    moved["details"] = {
        "target_text": target,
        "text": details.get("text"),
        "source": source,
        "occurrence": occurrence,
        "x": x,
        "y": y,
        "offset_x": offset_x,
        "offset_y": offset_y,
        "match_count": details.get("match_count") or 0,
        "warnings": details.get("warnings") or [],
    }
    return moved


def assert_text(args, state_dir):
    expected_exists = bool_arg(args, "exists", True)
    outcome = find_text(args, state_dir)
    details = outcome.get("details")
    if not isinstance(details, dict):
        return outcome
    found = int(details.get("match_count") or 0) > 0
    details = {**details, "exists": expected_exists, "found": found}
    text = (
        "Computer action: assert_text\n"
        f"Query: {details.get('text')}\n"
        f"Exists: {str(expected_exists).lower()}\n"
        f"Found: {str(found).lower()}\n"
        f"Matches: {json.dumps(details.get('matches') or [], ensure_ascii=False)}"
    )
    if details.get("warnings"):
        text += f"\nWarnings: {json.dumps(details.get('warnings'), ensure_ascii=False)}"
    if found == expected_exists:
        result = ok(text, "text condition matched")
    else:
        result = err(text, f"expected text found={str(expected_exists).lower()}")
    result["details"] = details
    if outcome.get("screenshot"):
        result["screenshot"] = outcome.get("screenshot")
    return result


def wait_for_text(args, state_dir):
    timeout_ms = int_arg(args, "timeout_ms", 5000, 0, 60000)
    interval_ms = int_arg(args, "interval_ms", 500, 100, 5000)
    expected_exists = bool_arg(args, "exists", True)
    started = time.monotonic()
    deadline = started + (timeout_ms / 1000)
    attempts = 0
    last = None
    while True:
        attempts += 1
        attempt_args = dict(args)
        attempt_args["screenshot"] = False
        last = assert_text(attempt_args, state_dir)
        details = last.get("details")
        if not isinstance(details, dict):
            return last
        if last.get("ok"):
            details = {
                **details,
                "attempts": attempts,
                "elapsed_ms": int((time.monotonic() - started) * 1000),
                "timeout_ms": timeout_ms,
                "interval_ms": interval_ms,
            }
            text = (
                "Computer action: wait_for_text\n"
                f"Query: {details.get('text')}\n"
                f"Exists: {str(expected_exists).lower()}\n"
                f"Attempts: {attempts}\n"
                f"Matches: {json.dumps(details.get('matches') or [], ensure_ascii=False)}"
            )
            result = ok(text, "text condition matched")
            result["details"] = details
            return result
        now = time.monotonic()
        if now >= deadline:
            break
        time.sleep(min(interval_ms / 1000, max(0, deadline - now)))
    details = last.get("details") if isinstance(last, dict) and isinstance(last.get("details"), dict) else {}
    details = {
        **details,
        "attempts": attempts,
        "elapsed_ms": int((time.monotonic() - started) * 1000),
        "timeout_ms": timeout_ms,
        "interval_ms": interval_ms,
    }
    query = str(args.get("text") or "").strip()
    text = (
        "Computer action: wait_for_text\n"
        f"Query: {query}\n"
        f"Exists: {str(expected_exists).lower()}\n"
        f"Attempts: {attempts}\n"
        f"Last matches: {json.dumps(details.get('matches') or [], ensure_ascii=False)}"
    )
    result = err(text, f'timed out waiting for text "{query}"')
    result["details"] = details
    return result


def screenshot_signature(path):
    data = Path(path).read_bytes()
    return {
        "sha256": hashlib.sha256(data).hexdigest(),
        "byte_count": len(data),
        "pixel_size": png_dimensions(path),
    }


def stable_screenshot_args(args):
    screenshot_args = {
        key: args.get(key)
        for key in ("x", "y", "width", "height", "display_index", "display_id", "path")
        if args.get(key) is not None and args.get(key) != ""
    }
    screenshot_args["timeout_sec"] = int_arg(args, "timeout_sec", 20, 1, 120)
    return screenshot_args


def wait_for_stable_screenshot(args, state_dir):
    timeout_ms = int_arg(args, "timeout_ms", 5000, 0, 60000)
    stable_ms = int_arg(args, "stable_ms", 1000, 100, 10000)
    interval_ms = int_arg(args, "interval_ms", 500, 100, 5000)
    screenshot_args = stable_screenshot_args(args)
    deadline = time.monotonic() + (timeout_ms / 1000)
    attempts = 0
    last_signature = None
    last_changed_at = time.monotonic()
    last_path = None

    while True:
        attempts += 1
        path = take_screenshot(screenshot_args, state_dir)
        now = time.monotonic()
        signature = screenshot_signature(path)
        last_path = path
        if last_signature is None or signature.get("sha256") != last_signature.get("sha256"):
            last_signature = signature
            last_changed_at = now
        stable_for_ms = int(max(0, (now - last_changed_at) * 1000))
        if stable_for_ms >= stable_ms:
            return {
                "stable": True,
                "attempts": attempts,
                "stable_for_ms": stable_for_ms,
                "timeout_ms": timeout_ms,
                "stable_ms": stable_ms,
                "interval_ms": interval_ms,
                "screenshot": str(last_path),
                "signature": last_signature,
                "screenshot_pixel_size": png_dimensions(last_path),
            }
        if now >= deadline:
            return {
                "stable": False,
                "attempts": attempts,
                "stable_for_ms": stable_for_ms,
                "timeout_ms": timeout_ms,
                "stable_ms": stable_ms,
                "interval_ms": interval_ms,
                "screenshot": str(last_path),
                "signature": last_signature or {},
                "screenshot_pixel_size": png_dimensions(last_path) if last_path else {},
            }
        time.sleep(min(interval_ms / 1000, max(0, deadline - now)))


def wait_for_desktop(args, state_dir):
    duration_ms = int_arg(args, "duration_ms", 1000, 0, 30000)
    until_stable = bool_arg(args, "until_stable", False)
    if duration_ms:
        time.sleep(duration_ms / 1000)
    stable = wait_for_stable_screenshot(args, state_dir) if until_stable else None
    text = (
        "Computer action: wait\n"
        f"Waited ms: {duration_ms}\n"
        f"Screen size: {screen_size() or '(unknown)'}"
    )
    if stable is not None:
        text += f"\nStability: {json.dumps(stable, ensure_ascii=False)}"
    if args.get("screenshot", True):
        if stable and stable.get("screenshot"):
            shot = Path(str(stable.get("screenshot")))
        else:
            shot = take_screenshot(stable_screenshot_args(args), state_dir)
        text += f"\nScreenshot: {shot}"
        text += f"\nScreenshot pixels: {json.dumps(png_dimensions(shot), ensure_ascii=False)}"
    display = "waited for stable desktop" if stable and stable.get("stable") else "waited for desktop"
    return ok(text, display)


def parse_accessibility_snapshot(output, limit):
    lines = [line for line in str(output or "").splitlines() if line.strip()]
    if not lines:
        return {}, []
    header = (lines[0].split("\t") + ["", ""])[:2]
    meta = {"frontmost_app": header[0], "front_window": header[1]}
    elements = []
    for raw_line in lines[1:]:
        parts = (raw_line.split("\t") + [""] * 10)[:10]
        try:
            index = int(parts[0])
        except (TypeError, ValueError):
            index = len(elements) + 1
        item = {
            "index": index,
            "role": parts[1],
            "title": parts[2],
            "description": parts[3],
        }
        if parts[4]:
            item["value"] = parts[4]
        if parts[5]:
            item["enabled"] = parts[5].strip().lower() == "true"
        if parts[6]:
            try:
                x_text, y_text, width_text, height_text = parts[6].split(",", 3)
                x = int(float(x_text))
                y = int(float(y_text))
                width = int(float(width_text))
                height = int(float(height_text))
                item.update({
                    "x": x,
                    "y": y,
                    "width": width,
                    "height": height,
                    "center_x": x + (width / 2),
                    "center_y": y + (height / 2),
                })
            except (TypeError, ValueError):
                item["bounds"] = parts[6]
        if parts[7]:
            actions = [action.strip() for action in parts[7].split(",") if action.strip()]
            if actions:
                item["actions"] = actions
        if parts[8]:
            item["focused"] = parts[8].strip().lower() == "true"
        if parts[9]:
            item["selected"] = parts[9].strip().lower() == "true"
        elements.append({key: value for key, value in item.items() if value not in ("", None)})
        if len(elements) >= limit:
            break
    return meta, elements


def accessibility_snapshot(args, state_dir):
    ensure_macos("accessibility snapshot")
    meta, elements = accessibility_snapshot_elements(args)
    text = (
        "Computer action: accessibility_snapshot\n"
        f"Frontmost app: {meta.get('frontmost_app') or '(unknown)'}\n"
        f"Front window: {meta.get('front_window') or ''}\n"
        f"Elements: {json.dumps(elements, ensure_ascii=False)}\n"
        f"Screen size: {screen_size() or '(unknown)'}"
    )
    return ok(text, f"captured {len(elements)} accessibility elements")


def accessibility_snapshot_elements(args):
    limit = int_arg(args, "max_elements", 80, 1, 500)
    include_values = bool_arg(args, "include_values", False)
    include_actions = bool_arg(args, "include_actions", False)
    include_state = bool_arg(args, "include_state", False)
    include_values_script = "true" if include_values else "false"
    include_actions_script = "true" if include_actions else "false"
    include_state_script = "true" if include_state else "false"
    script = f'''
on cleanText(valueToClean)
    try
        set textValue to valueToClean as text
    on error
        return ""
    end try
    set previousDelimiters to AppleScript's text item delimiters
    set AppleScript's text item delimiters to {{tab, linefeed, return}}
    set textParts to text items of textValue
    set AppleScript's text item delimiters to " "
    set textValue to textParts as text
    set AppleScript's text item delimiters to previousDelimiters
    if (length of textValue) > 160 then set textValue to text 1 thru 160 of textValue
    return textValue
end cleanText

on joinTextValues(valueList)
    set previousDelimiters to AppleScript's text item delimiters
    set AppleScript's text item delimiters to ","
    set joinedText to valueList as text
    set AppleScript's text item delimiters to previousDelimiters
    return joinedText
end joinTextValues

set maxItems to {limit}
set includeValues to {include_values_script}
set includeActions to {include_actions_script}
set includeState to {include_state_script}
set previousDelimiters to AppleScript's text item delimiters
set AppleScript's text item delimiters to linefeed
set rows to {{}}
set appName to ""
set windowName to ""
tell application "System Events"
    set frontProc to first application process whose frontmost is true
    set appName to name of frontProc
    try
        set windowName to my cleanText(name of window 1 of frontProc)
    end try
    set elementsList to entire contents of frontProc
    set elementIndex to 0
    repeat with uiElement in elementsList
        if (count of rows) is greater than or equal to maxItems then exit repeat
        set roleText to ""
        set titleText to ""
        set descriptionText to ""
        set valueText to ""
        set enabledText to ""
        set boundsText to ""
        set actionsText to ""
        set focusedText to ""
        set selectedText to ""
        try
            set roleText to my cleanText(role of uiElement)
        end try
        try
            set titleText to my cleanText(name of uiElement)
        end try
        try
            set descriptionText to my cleanText(description of uiElement)
        end try
        if includeValues then
            try
                set valueText to my cleanText(value of uiElement)
            end try
        end if
        try
            set enabledText to enabled of uiElement as text
        end try
        try
            set elementPosition to position of uiElement
            set elementSize to size of uiElement
            set boundsText to ((item 1 of elementPosition) as text) & "," & ((item 2 of elementPosition) as text) & "," & ((item 1 of elementSize) as text) & "," & ((item 2 of elementSize) as text)
        end try
        if includeActions then
            try
                set actionNames to {{}}
                repeat with actionItem in actions of uiElement
                    try
                        set end of actionNames to my cleanText(name of actionItem)
                    end try
                end repeat
                if (count of actionNames) > 0 then set actionsText to my joinTextValues(actionNames)
            end try
        end if
        if includeState then
            try
                set focusedText to focused of uiElement as text
            end try
            try
                set selectedText to selected of uiElement as text
            end try
        end if
        if roleText is not "" or titleText is not "" or descriptionText is not "" or valueText is not "" or boundsText is not "" then
            set elementIndex to elementIndex + 1
            set end of rows to (elementIndex as text) & tab & roleText & tab & titleText & tab & descriptionText & tab & valueText & tab & enabledText & tab & boundsText & tab & actionsText & tab & focusedText & tab & selectedText
        end if
    end repeat
end tell
set outputText to appName & tab & windowName
if (count of rows) > 0 then set outputText to outputText & linefeed & (rows as text)
set AppleScript's text item delimiters to previousDelimiters
return outputText
'''
    output = run_osascript(script, timeout=20)
    meta, elements = parse_accessibility_snapshot(output, limit)
    return meta, elements


class CGPoint(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]


def app_services():
    path = ctypes.util.find_library("ApplicationServices") or (
        "/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices"
    )
    if not path:
        raise RuntimeError("ApplicationServices framework is unavailable")
    return ctypes.CDLL(path)


def cf_release(ptr):
    try:
        cf = ctypes.CDLL(ctypes.util.find_library("CoreFoundation") or "CoreFoundation")
        cf.CFRelease.argtypes = [ctypes.c_void_p]
        cf.CFRelease(ptr)
    except Exception:
        pass


def button_events(button):
    value = str(button or "left").lower()
    if value == "left":
        return 0, 1, 2
    if value == "right":
        return 1, 3, 4
    if value == "middle":
        return 2, 25, 26
    raise RuntimeError("button must be left/right/middle")


MOUSE_MODIFIER_FLAGS = {
    "Shift": 1 << 17,
    "Control": 1 << 18,
    "Alt": 1 << 19,
    "Option": 1 << 19,
    "Meta": 1 << 20,
    "Command": 1 << 20,
}
MOUSE_MODIFIER_ALIASES = {
    "Alt": "Option",
    "Meta": "Command",
}


def mouse_modifier_flags(raw_modifiers):
    if raw_modifiers is None or raw_modifiers == "":
        return 0, []
    if not isinstance(raw_modifiers, list):
        raise RuntimeError("modifiers must be an array")
    if len(raw_modifiers) > 4:
        raise RuntimeError("modifiers must contain at most 4 items")
    flags = 0
    labels = []
    seen = set()
    for raw_modifier in raw_modifiers:
        modifier = str(raw_modifier or "").strip()
        if modifier not in MOUSE_MODIFIER_FLAGS:
            raise RuntimeError("modifier is not supported")
        canonical = MOUSE_MODIFIER_ALIASES.get(modifier, modifier)
        if canonical in seen:
            continue
        seen.add(canonical)
        labels.append(canonical)
        flags |= MOUSE_MODIFIER_FLAGS[modifier]
    return flags, labels


def coord(args, key):
    try:
        value = float(args.get(key))
    except (TypeError, ValueError) as exc:
        raise RuntimeError(f"{key} must be a number") from exc
    if value < MIN_SCREEN_COORDINATE or value > MAX_SCREEN_COORDINATE:
        raise RuntimeError(f"{key} must be between {MIN_SCREEN_COORDINATE} and {MAX_SCREEN_COORDINATE}")
    return value


def element_offset(args, key):
    try:
        raw = args.get(key)
        value = float(0 if raw is None or raw == "" else raw)
    except (TypeError, ValueError) as exc:
        raise RuntimeError(f"{key} must be a number") from exc
    if value < MIN_ELEMENT_OFFSET or value > MAX_ELEMENT_OFFSET:
        raise RuntimeError(f"{key} must be between {MIN_ELEMENT_OFFSET} and {MAX_ELEMENT_OFFSET}")
    return value


def element_offset_default(args, key, default):
    raw = args.get(key)
    if raw is None or raw == "":
        return default
    return element_offset(args, key)


def post_mouse(cg, event_type, point, button, flags=0, click_count=0):
    cg.CGEventCreateMouseEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint32, CGPoint, ctypes.c_uint32]
    cg.CGEventCreateMouseEvent.restype = ctypes.c_void_p
    cg.CGEventSetFlags.argtypes = [ctypes.c_void_p, ctypes.c_uint64]
    set_integer = getattr(cg, "CGEventSetIntegerValueField", None)
    if set_integer is not None:
        set_integer.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_int64]
    cg.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
    event = cg.CGEventCreateMouseEvent(None, event_type, point, button)
    if not event:
        raise RuntimeError("failed to create mouse event")
    try:
        if flags:
            cg.CGEventSetFlags(event, flags)
        if set_integer is not None and click_count > 0:
            set_integer(event, CG_MOUSE_EVENT_CLICK_STATE_FIELD, int(click_count))
        cg.CGEventPost(0, event)
    finally:
        cf_release(event)


def post_keyboard(cg, key_code, pressed):
    cg.CGEventCreateKeyboardEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint16, ctypes.c_bool]
    cg.CGEventCreateKeyboardEvent.restype = ctypes.c_void_p
    cg.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
    event = cg.CGEventCreateKeyboardEvent(None, key_code, bool(pressed))
    if not event:
        raise RuntimeError("failed to create keyboard event")
    try:
        cg.CGEventPost(0, event)
    finally:
        cf_release(event)


def current_position():
    ensure_macos("cursor position")
    cg = app_services()
    cg.CGEventCreate.argtypes = [ctypes.c_void_p]
    cg.CGEventCreate.restype = ctypes.c_void_p
    cg.CGEventGetLocation.argtypes = [ctypes.c_void_p]
    cg.CGEventGetLocation.restype = CGPoint
    event = cg.CGEventCreate(None)
    if not event:
        raise RuntimeError("failed to read mouse position")
    try:
        point = cg.CGEventGetLocation(event)
    finally:
        cf_release(event)
    return float(point.x), float(point.y)


def get_position(args, state_dir):
    x, y = current_position()
    text = (
        "Computer action: get_position\n"
        f"Cursor: {x:g},{y:g}\n"
        f"Screen size: {screen_size() or '(unknown)'}"
    )
    return ok(text, "read cursor position")


def move_mouse(args, state_dir):
    ensure_macos("move mouse")
    x = coord(args, "x")
    y = coord(args, "y")
    wait_after_ms = int_arg(args, "wait_after_ms", 250, 0, 5000)
    cg = app_services()
    post_mouse(cg, 5, CGPoint(x, y), 0)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    text = f"Computer action: move_mouse\nMoved: {x:g},{y:g}\nWait after ms: {wait_after_ms}"
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, "moved host mouse")


def optional_mouse_point(args):
    has_x = args.get("x") is not None and args.get("x") != ""
    has_y = args.get("y") is not None and args.get("y") != ""
    if has_x != has_y:
        raise RuntimeError("x and y must be provided together")
    if has_x:
        return coord(args, "x"), coord(args, "y")
    return current_position()


def mouse_button_state(args, state_dir, pressed):
    ensure_macos("mouse down" if pressed else "mouse up")
    x, y = optional_mouse_point(args)
    wait_after_ms = int_arg(args, "wait_after_ms", 250, 0, 5000)
    button, down, up = button_events(args.get("button"))
    modifier_flags, modifiers = mouse_modifier_flags(args.get("modifiers"))
    cg = app_services()
    point = CGPoint(x, y)
    post_mouse(cg, 5, point, button, modifier_flags)
    post_mouse(cg, down if pressed else up, point, button, modifier_flags)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    action = "mouse_down" if pressed else "mouse_up"
    state = "Pressed" if pressed else "Released"
    text = (
        f"Computer action: {action}\n"
        f"{state}: {x:g},{y:g}\n"
        f"Button: {str(args.get('button') or 'left').lower()}\n"
        f"Wait after ms: {wait_after_ms}"
    )
    if modifiers:
        text += f"\nModifiers: {', '.join(modifiers)}"
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, "pressed host mouse" if pressed else "released host mouse")


def mouse_down(args, state_dir):
    return mouse_button_state(args, state_dir, True)


def mouse_up(args, state_dir):
    return mouse_button_state(args, state_dir, False)


def click(args, state_dir):
    ensure_macos("click")
    x = coord(args, "x")
    y = coord(args, "y")
    clicks = int_arg(args, "clicks", 1, 1, 3)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    button, down, up = button_events(args.get("button"))
    modifier_flags, modifiers = mouse_modifier_flags(args.get("modifiers"))
    cg = app_services()
    point = CGPoint(x, y)
    for click_index in range(1, clicks + 1):
        post_mouse(cg, down, point, button, modifier_flags, click_count=click_index)
        post_mouse(cg, up, point, button, modifier_flags, click_count=click_index)
        time.sleep(0.08)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    text = f"Computer action: click\nClicked: {x:g},{y:g}\nClicks: {clicks}"
    if modifiers:
        text += f"\nModifiers: {', '.join(modifiers)}"
    text += f"\nWait after ms: {wait_after_ms}"
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, "clicked host screen")


def element_text_match(value, query, match_mode):
    text = str(value or "").strip().lower()
    if not text:
        return False
    query = query.lower()
    if match_mode == "exact":
        return text == query
    return query in text


def element_matches(element, query, role, match_mode):
    if role and role.lower() not in str(element.get("role") or "").lower():
        return False
    if not query:
        return False
    return any(
        element_text_match(element.get(field), query, match_mode)
        for field in ("title", "description", "value")
    )


def optional_element_index(args, key="index"):
    raw = args.get(key)
    if raw is None or raw == "":
        return None
    try:
        value = int(raw)
    except (TypeError, ValueError):
        raise RuntimeError(f"{key} must be an integer")
    if value < 1 or value > 500:
        raise RuntimeError(f"{key} must be between 1 and 500")
    return value


def element_matches_target(element, query, index, role, match_mode):
    if role and role.lower() not in str(element.get("role") or "").lower():
        return False
    if index is not None:
        try:
            if int(element.get("index")) != index:
                return False
        except (TypeError, ValueError):
            return False
        if not query:
            return True
    return element_matches(element, query, "", match_mode)


def element_target_label(query, index):
    if query and index is not None:
        return f'"{query}" at index {index}'
    if index is not None:
        return f"index {index}"
    return f'"{query}"'


def element_center(element):
    if element.get("center_x") is not None and element.get("center_y") is not None:
        try:
            return float(element["center_x"]), float(element["center_y"])
        except (TypeError, ValueError):
            return None
    if all(element.get(key) is not None for key in ("x", "y", "width", "height")):
        try:
            x = float(element["x"])
            y = float(element["y"])
            width = float(element["width"])
            height = float(element["height"])
            return x + (width / 2), y + (height / 2)
        except (TypeError, ValueError):
            return None
    return None


def element_bounds(element):
    if not all(element.get(key) is not None for key in ("x", "y", "width", "height")):
        return None
    try:
        x = float(element["x"])
        y = float(element["y"])
        width = float(element["width"])
        height = float(element["height"])
    except (TypeError, ValueError):
        return None
    if width <= 0 or height <= 0:
        return None
    return x, y, width, height


def element_contains_point(element, x, y):
    bounds = element_bounds(element)
    if bounds is None:
        return None
    left, top, width, height = bounds
    right = left + width
    bottom = top + height
    if x < left or x > right or y < top or y > bottom:
        return None
    return width * height, {"x": left, "y": top, "width": width, "height": height}


def element_at(args, state_dir):
    ensure_macos("element at point")
    x = coord(args, "x")
    y = coord(args, "y")
    max_elements = int_arg(args, "max_elements", 200, 1, 500)
    include_values = bool_arg(args, "include_values", True)
    include_actions = bool_arg(args, "include_actions", True)
    include_state = bool_arg(args, "include_state", True)
    meta, elements = accessibility_snapshot_elements({
        "max_elements": max_elements,
        "include_values": include_values,
        "include_actions": include_actions,
        "include_state": include_state,
    })
    matches = []
    for element in elements:
        contained = element_contains_point(element, x, y)
        if contained is None:
            continue
        area, bounds = contained
        matches.append({"area": area, "bounds": bounds, "element": element})
    matches.sort(key=lambda item: (float(item.get("area") or 0), int(item.get("element", {}).get("index") or 0)))
    if not matches:
        details = {
            "x": x,
            "y": y,
            "max_elements": max_elements,
            "candidate_elements": candidate_summary(elements),
            "frontmost_app": meta.get("frontmost_app"),
            "front_window": meta.get("front_window"),
        }
        outcome = err(
            "Computer action: element_at\n"
            f"Point: {x:g},{y:g}\n"
            "Matched element: none\n"
            f"Candidate elements: {json.dumps(details['candidate_elements'], ensure_ascii=False)}",
            f"no element at {x:g},{y:g}",
        )
        outcome["details"] = details
        return outcome

    matched = matches[0]
    element = matched.get("element") or {}
    details = {
        "x": x,
        "y": y,
        "matched_element": element,
        "matched_bounds": matched.get("bounds") or {},
        "match_count": len(matches),
        "matches": matches[:10],
        "frontmost_app": meta.get("frontmost_app"),
        "front_window": meta.get("front_window"),
        "max_elements": max_elements,
        "include_values": include_values,
        "include_actions": include_actions,
        "include_state": include_state,
    }
    text = (
        "Computer action: element_at\n"
        f"Point: {x:g},{y:g}\n"
        f"Matched element: {json.dumps(element, ensure_ascii=False)}\n"
        f"Matches: {json.dumps(matches[:10], ensure_ascii=False)}"
    )
    outcome = ok(text, f"found element at {x:g},{y:g}")
    outcome["details"] = details
    return outcome


def press_at(args, state_dir):
    ensure_macos("press element at point")
    x = coord(args, "x")
    y = coord(args, "y")
    max_elements = int_arg(args, "max_elements", 200, 1, 500)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    screenshot_enabled = bool_arg(args, "screenshot", True)
    meta, elements = accessibility_snapshot_elements({
        "max_elements": max_elements,
        "include_values": True,
        "include_actions": True,
        "include_state": True,
    })
    matches = []
    for element in elements:
        contained = element_contains_point(element, x, y)
        if contained is None:
            continue
        area, bounds = contained
        matches.append({"area": area, "bounds": bounds, "element": element})
    matches.sort(key=lambda item: (float(item.get("area") or 0), int(item.get("element", {}).get("index") or 0)))
    if not matches:
        details = {
            "x": x,
            "y": y,
            "max_elements": max_elements,
            "candidate_elements": candidate_summary(elements),
            "frontmost_app": meta.get("frontmost_app"),
            "front_window": meta.get("front_window"),
        }
        outcome = err(
            "Computer action: press_at\n"
            f"Point: {x:g},{y:g}\n"
            "Matched element: none\n"
            f"Candidate elements: {json.dumps(details['candidate_elements'], ensure_ascii=False)}",
            f"no element at {x:g},{y:g}",
        )
        outcome["details"] = details
        return outcome

    matched = matches[0]
    element = matched.get("element") or {}
    try:
        pressed_index = int(element.get("index"))
    except (TypeError, ValueError) as exc:
        raise RuntimeError("matched accessibility element has no stable index") from exc
    method = press_accessibility_element_by_index(pressed_index)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000.0)
    details = {
        "x": x,
        "y": y,
        "pressed_index": pressed_index,
        "method": method,
        "matched_element": element,
        "matched_bounds": matched.get("bounds") or {},
        "match_count": len(matches),
        "matches": matches[:10],
        "frontmost_app": meta.get("frontmost_app"),
        "front_window": meta.get("front_window"),
        "max_elements": max_elements,
        "wait_after_ms": wait_after_ms,
    }
    text = (
        "Computer action: press_at\n"
        f"Point: {x:g},{y:g}\n"
        f"Pressed index: {pressed_index}\n"
        f"Method: {method}\n"
        f"Matched element: {json.dumps(element, ensure_ascii=False)}"
    )
    if screenshot_enabled:
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    outcome = ok(text, f"pressed element at {x:g},{y:g}")
    outcome["details"] = details
    return outcome


def type_at(args, state_dir):
    ensure_macos("type at point")
    x = coord(args, "x")
    y = coord(args, "y")
    text_value = args.get("text")
    if not isinstance(text_value, str):
        raise RuntimeError("text is required")
    if len(text_value) > 10000:
        raise RuntimeError("text must be <= 10000 characters")
    max_elements = int_arg(args, "max_elements", 200, 1, 500)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    paste_settle_ms = int_arg(args, "paste_settle_ms", DEFAULT_PASTE_SETTLE_MS, 0, 2000)
    paste = bool_arg(args, "paste", True)
    clear = bool_arg(args, "clear", True)
    press_enter = bool_arg(args, "press_enter", False)
    screenshot_enabled = bool_arg(args, "screenshot", True)
    meta, elements = accessibility_snapshot_elements({
        "max_elements": max_elements,
        "include_values": True,
        "include_actions": True,
        "include_state": True,
    })
    matches = []
    for element in elements:
        contained = element_contains_point(element, x, y)
        if contained is None:
            continue
        area, bounds = contained
        matches.append({"area": area, "bounds": bounds, "element": element})
    matches.sort(key=lambda item: (float(item.get("area") or 0), int(item.get("element", {}).get("index") or 0)))
    if not matches:
        details = {
            "x": x,
            "y": y,
            "max_elements": max_elements,
            "candidate_elements": candidate_summary(elements),
            "frontmost_app": meta.get("frontmost_app"),
            "front_window": meta.get("front_window"),
        }
        outcome = err(
            "Computer action: type_at\n"
            f"Point: {x:g},{y:g}\n"
            "Matched element: none\n"
            f"Candidate elements: {json.dumps(details['candidate_elements'], ensure_ascii=False)}",
            f"no element at {x:g},{y:g}",
        )
        outcome["details"] = details
        return outcome

    matched = matches[0]
    element = matched.get("element") or {}
    cg = app_services()
    point = CGPoint(x, y)
    _, down, up = button_events("left")
    post_mouse(cg, down, point, 0, click_count=1)
    post_mouse(cg, up, point, 0, click_count=1)
    time.sleep(0.08)
    if clear:
        clear_focused_text()
    if paste:
        paste_text_with_clipboard(text_value, settle_ms=paste_settle_ms, timeout=8)
    else:
        run_osascript(f'tell application "System Events" to keystroke "{quote_applescript(text_value)}"', timeout=20)
    if press_enter:
        press_enter_key()
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000.0)
    details = {
        "x": x,
        "y": y,
        "typed_x": x,
        "typed_y": y,
        "typed_characters": len(text_value),
        "paste": paste,
        "paste_settle_ms": paste_settle_ms,
        "clear": clear,
        "press_enter": press_enter,
        "wait_after_ms": wait_after_ms,
        "matched_element": element,
        "matched_bounds": matched.get("bounds") or {},
        "match_count": len(matches),
        "matches": matches[:10],
        "frontmost_app": meta.get("frontmost_app"),
        "front_window": meta.get("front_window"),
        "max_elements": max_elements,
    }
    text = (
        "Computer action: type_at\n"
        f"Point: {x:g},{y:g}\n"
        f"Typed characters: {len(text_value)}\n"
        f"Paste: {str(paste).lower()}\n"
        f"Clear: {str(clear).lower()}\n"
        f"Press enter: {str(press_enter).lower()}\n"
        f"Matched element: {json.dumps(element, ensure_ascii=False)}"
    )
    if screenshot_enabled:
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    outcome = ok(text, f"typed at {x:g},{y:g}")
    outcome["details"] = details
    return outcome


def scroll_at(args, state_dir):
    ensure_macos("scroll at point")
    x = coord(args, "x")
    y = coord(args, "y")
    max_elements = int_arg(args, "max_elements", 200, 1, 500)
    meta, elements = accessibility_snapshot_elements({
        "max_elements": max_elements,
        "include_values": True,
        "include_actions": True,
        "include_state": True,
    })
    matches = []
    for element in elements:
        contained = element_contains_point(element, x, y)
        if contained is None:
            continue
        area, bounds = contained
        matches.append({"area": area, "bounds": bounds, "element": element})
    matches.sort(key=lambda item: (float(item.get("area") or 0), int(item.get("element", {}).get("index") or 0)))
    if not matches:
        details = {
            "x": x,
            "y": y,
            "max_elements": max_elements,
            "candidate_elements": candidate_summary(elements),
            "frontmost_app": meta.get("frontmost_app"),
            "front_window": meta.get("front_window"),
        }
        outcome = err(
            "Computer action: scroll_at\n"
            f"Point: {x:g},{y:g}\n"
            "Matched element: none\n"
            f"Candidate elements: {json.dumps(details['candidate_elements'], ensure_ascii=False)}",
            f"no element at {x:g},{y:g}",
        )
        outcome["details"] = details
        return outcome

    matched = matches[0]
    element = matched.get("element") or {}
    scroll_args = dict(args)
    scroll_args["x"] = x
    scroll_args["y"] = y
    result = scroll(scroll_args, state_dir)
    result["text"] = (
        "Computer action: scroll_at\n"
        f"Point: {x:g},{y:g}\n"
        f"Matched element: {json.dumps(element, ensure_ascii=False)}\n"
        + "\n".join(str(result.get("text") or "").splitlines()[1:])
    )
    details = {
        "x": x,
        "y": y,
        "scrolled_x": x,
        "scrolled_y": y,
        "matched_element": element,
        "matched_bounds": matched.get("bounds") or {},
        "match_count": len(matches),
        "matches": matches[:10],
        "frontmost_app": meta.get("frontmost_app"),
        "front_window": meta.get("front_window"),
        "max_elements": max_elements,
    }
    result.setdefault("details", {})
    if isinstance(result.get("details"), dict):
        result["details"].update(details)
    else:
        result["details"] = details
    result["display"] = f"scrolled at {x:g},{y:g}"
    return result


def element_point(element, offset_x=0, offset_y=0):
    center = element_center(element)
    if center is None:
        return None
    return center[0] + offset_x, center[1] + offset_y


def candidate_summary(elements):
    summary = []
    for element in elements[:12]:
        summary.append({
            key: element.get(key)
            for key in ("index", "role", "title", "description", "value", "enabled", "focused", "selected")
            if element.get(key) not in ("", None)
        })
    return summary


def screenshot_element(args, state_dir):
    ensure_macos("screenshot element")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    target_label = element_target_label(query, index)
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    padding = int_arg(args, "padding", 0, 0, 5000)
    timeout_sec = int_arg(args, "timeout_sec", 20, 1, 120)
    _, elements = accessibility_snapshot_elements({"max_elements": max_elements, "include_values": True})

    matches = []
    unbounded = None
    for element in elements:
        if not element_matches_target(element, query, index, role, match_mode):
            continue
        bounds = element_bounds(element)
        if bounds is None:
            if unbounded is None:
                unbounded = element
            continue
        matches.append(element)
        if len(matches) < occurrence:
            continue
        x, y, width, height = bounds
        screenshot_args = dict(args)
        screenshot_args["x"] = int(round(x - padding))
        screenshot_args["y"] = int(round(y - padding))
        screenshot_args["width"] = int(round(width + (padding * 2)))
        screenshot_args["height"] = int(round(height + (padding * 2)))
        screenshot_args["timeout_sec"] = timeout_sec
        result = screenshot_result(screenshot_args, state_dir, action="screenshot_element")
        result["text"] = (
            "Computer action: screenshot_element\n"
            f"Target element: {target_label}\n"
            f"Element bounds: {x:g},{y:g},{width:g},{height:g}\n"
            f"Padding: {padding}\n"
            f"Matched element: {json.dumps(element, ensure_ascii=False)}\n"
            + "\n".join(str(result.get("text") or "").splitlines()[1:])
        )
        result["display"] = f"screenshot element {target_label}"
        return result
    if unbounded is not None:
        raise RuntimeError("matching accessibility element has no bounds")
    raise RuntimeError(f"no accessibility element matched {target_label}")


def screenshot_window(args, state_dir):
    ensure_macos("screenshot window")
    padding = int_arg(args, "padding", 0, 0, 5000)
    timeout_sec = int_arg(args, "timeout_sec", 20, 1, 120)
    matches, matched = find_window_match(args, state_dir)
    if matched.get("minimized") is True:
        raise RuntimeError("matched window is minimized; restore it before taking a window screenshot")
    try:
        x = float(matched["x"])
        y = float(matched["y"])
        width = float(matched["width"])
        height = float(matched["height"])
    except (KeyError, TypeError, ValueError) as exc:
        raise RuntimeError("matched window has no usable bounds") from exc
    if width <= 0 or height <= 0:
        raise RuntimeError("matched window has invalid bounds")
    screenshot_args = dict(args)
    screenshot_args["x"] = int(round(x - padding))
    screenshot_args["y"] = int(round(y - padding))
    screenshot_args["width"] = int(round(width + (padding * 2)))
    screenshot_args["height"] = int(round(height + (padding * 2)))
    screenshot_args["timeout_sec"] = timeout_sec
    result = screenshot_result(screenshot_args, state_dir, action="screenshot_window")
    result["text"] = (
        "Computer action: screenshot_window\n"
        f"Target window: {json.dumps(matched, ensure_ascii=False)}\n"
        f"Match count: {len(matches)}\n"
        f"Window bounds: {x:g},{y:g},{width:g},{height:g}\n"
        f"Padding: {padding}\n"
        + "\n".join(str(result.get("text") or "").splitlines()[1:])
    )
    result["display"] = f"screenshot window {matched.get('app') or ''} {matched.get('title') or ''}".strip()
    return result


def click_element(args, state_dir):
    ensure_macos("click element")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    target_label = element_target_label(query, index)
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    clicks = int_arg(args, "clicks", 1, 1, 3)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    offset_x = element_offset(args, "offset_x")
    offset_y = element_offset(args, "offset_y")
    button, down, up = button_events(args.get("button"))
    modifier_flags, modifiers = mouse_modifier_flags(args.get("modifiers"))
    _, elements = accessibility_snapshot_elements({"max_elements": max_elements, "include_values": True})

    matches = []
    unbounded = None
    for element in elements:
        if not element_matches_target(element, query, index, role, match_mode):
            continue
        point = element_point(element, offset_x, offset_y)
        if point is None:
            if unbounded is None:
                unbounded = element
            continue
        matches.append(element)
        if len(matches) < occurrence:
            continue
        x, y = point
        cg = app_services()
        point = CGPoint(x, y)
        for click_index in range(1, clicks + 1):
            post_mouse(cg, down, point, button, modifier_flags, click_count=click_index)
            post_mouse(cg, up, point, button, modifier_flags, click_count=click_index)
            time.sleep(0.08)
        if wait_after_ms:
            time.sleep(wait_after_ms / 1000)
        text = (
            "Computer action: click_element\n"
            f"Target: {target_label}\n"
            f"Clicked: {x:g},{y:g}\n"
            f"Offset: {offset_x:g},{offset_y:g}\n"
            f"Modifiers: {', '.join(modifiers) if modifiers else '(none)'}\n"
            f"Matched element: {json.dumps(element, ensure_ascii=False)}"
        )
        if args.get("screenshot", True):
            shot = take_screenshot({}, state_dir)
            text += f"\nScreenshot: {shot}"
        return ok(text, f"clicked element {target_label}")
    if unbounded is not None:
        raise RuntimeError("matching accessibility element has no bounds")
    raise RuntimeError(f"no accessibility element matched {target_label}")


def press_accessibility_element_by_index(index):
    if index < 1 or index > 500:
        raise RuntimeError("element index must be between 1 and 500")
    script = f'''
set targetIndex to {index}
tell application "System Events"
    set frontProc to first application process whose frontmost is true
    set elementIndex to 0
    repeat with uiElement in entire contents of frontProc
        set roleText to ""
        set titleText to ""
        set descriptionText to ""
        set valueText to ""
        set boundsText to ""
        try
            set roleText to role of uiElement as text
        end try
        try
            set titleText to name of uiElement as text
        end try
        try
            set descriptionText to description of uiElement as text
        end try
        try
            set valueText to value of uiElement as text
        end try
        try
            set elementPosition to position of uiElement
            set elementSize to size of uiElement
            set boundsText to ((item 1 of elementPosition) as text) & "," & ((item 2 of elementPosition) as text) & "," & ((item 1 of elementSize) as text) & "," & ((item 2 of elementSize) as text)
        end try
        if roleText is not "" or titleText is not "" or descriptionText is not "" or valueText is not "" or boundsText is not "" then
            set elementIndex to elementIndex + 1
            if elementIndex is targetIndex then
                try
                    perform action "AXPress" of uiElement
                    return "AXPress"
                on error axError
                    try
                        click uiElement
                        return "click"
                    on error clickError
                        error axError & " | click fallback failed: " & clickError
                    end try
                end try
            end if
        end if
    end repeat
end tell
error "accessibility element index was not found"
'''
    return (run_osascript(script, timeout=12).strip() or "AXPress")


def focus_accessibility_element_by_index(index):
    if index < 1 or index > 500:
        raise RuntimeError("element index must be between 1 and 500")
    script = f'''
set targetIndex to {index}
tell application "System Events"
    set frontProc to first application process whose frontmost is true
    set elementIndex to 0
    repeat with uiElement in entire contents of frontProc
        set roleText to ""
        set titleText to ""
        set descriptionText to ""
        set valueText to ""
        set boundsText to ""
        try
            set roleText to role of uiElement as text
        end try
        try
            set titleText to name of uiElement as text
        end try
        try
            set descriptionText to description of uiElement as text
        end try
        try
            set valueText to value of uiElement as text
        end try
        try
            set elementPosition to position of uiElement
            set elementSize to size of uiElement
            set boundsText to ((item 1 of elementPosition) as text) & "," & ((item 2 of elementPosition) as text) & "," & ((item 1 of elementSize) as text) & "," & ((item 2 of elementSize) as text)
        end try
        if roleText is not "" or titleText is not "" or descriptionText is not "" or valueText is not "" or boundsText is not "" then
            set elementIndex to elementIndex + 1
            if elementIndex is targetIndex then
                try
                    set focused of uiElement to true
                    return "AXFocused"
                on error focusError
                    try
                        perform action "AXRaise" of uiElement
                    end try
                    try
                        set focused of uiElement to true
                        return "AXRaise+AXFocused"
                    on error retryError
                        error focusError & " | retry failed: " & retryError
                    end try
                end try
            end if
        end if
    end repeat
end tell
error "accessibility element index was not found"
'''
    return (run_osascript(script, timeout=12).strip() or "AXFocused")


def perform_accessibility_element_action_by_index(index, action):
    if index < 1 or index > 500:
        raise RuntimeError("element index must be between 1 and 500")
    if action not in ALLOWED_ELEMENT_ACTIONS:
        raise RuntimeError("action is not allowed")
    script = f'''
set targetIndex to {index}
tell application "System Events"
    set frontProc to first application process whose frontmost is true
    set elementIndex to 0
    repeat with uiElement in entire contents of frontProc
        set roleText to ""
        set titleText to ""
        set descriptionText to ""
        set valueText to ""
        set boundsText to ""
        try
            set roleText to role of uiElement as text
        end try
        try
            set titleText to name of uiElement as text
        end try
        try
            set descriptionText to description of uiElement as text
        end try
        try
            set valueText to value of uiElement as text
        end try
        try
            set elementPosition to position of uiElement
            set elementSize to size of uiElement
            set boundsText to ((item 1 of elementPosition) as text) & "," & ((item 2 of elementPosition) as text) & "," & ((item 1 of elementSize) as text) & "," & ((item 2 of elementSize) as text)
        end try
        if roleText is not "" or titleText is not "" or descriptionText is not "" or valueText is not "" or boundsText is not "" then
            set elementIndex to elementIndex + 1
            if elementIndex is targetIndex then
                perform action "{action}" of uiElement
                return "{action}"
            end if
        end if
    end repeat
end tell
error "accessibility element index was not found"
'''
    return (run_osascript(script, timeout=12).strip() or action)


def set_accessibility_element_value_by_index(index, value, press_enter=False):
    if index < 1 or index > 500:
        raise RuntimeError("element index must be between 1 and 500")
    should_press_return = "true" if press_enter else "false"
    script = f'''
set targetIndex to {index}
set newValue to {applescript_string_expr(value)}
set shouldPressReturn to {should_press_return}
tell application "System Events"
    set frontProc to first application process whose frontmost is true
    set elementIndex to 0
    repeat with uiElement in entire contents of frontProc
        set roleText to ""
        set titleText to ""
        set descriptionText to ""
        set valueText to ""
        set boundsText to ""
        try
            set roleText to role of uiElement as text
        end try
        try
            set titleText to name of uiElement as text
        end try
        try
            set descriptionText to description of uiElement as text
        end try
        try
            set valueText to value of uiElement as text
        end try
        try
            set elementPosition to position of uiElement
            set elementSize to size of uiElement
            set boundsText to ((item 1 of elementPosition) as text) & "," & ((item 2 of elementPosition) as text) & "," & ((item 1 of elementSize) as text) & "," & ((item 2 of elementSize) as text)
        end try
        if roleText is not "" or titleText is not "" or descriptionText is not "" or valueText is not "" or boundsText is not "" then
            set elementIndex to elementIndex + 1
            if elementIndex is targetIndex then
                try
                    set focused of uiElement to true
                end try
                try
                    set value of uiElement to newValue
                    if shouldPressReturn then key code 36
                    return "AXValue"
                on error valueError
                    error valueError
                end try
            end if
        end if
    end repeat
end tell
error "accessibility element index was not found"
'''
    return (run_osascript(script, timeout=12).strip() or "AXValue")


def press_element(args, state_dir):
    ensure_macos("press element")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    target_label = element_target_label(query, index)
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    _, elements = accessibility_snapshot_elements({"max_elements": max_elements, "include_values": True})

    matches = []
    for element in elements:
        if not element_matches_target(element, query, index, role, match_mode):
            continue
        matches.append(element)
        if len(matches) < occurrence:
            continue
        try:
            pressed_index = int(element.get("index"))
        except (TypeError, ValueError) as exc:
            raise RuntimeError("matching accessibility element has no stable index") from exc
        method = press_accessibility_element_by_index(pressed_index)
        if wait_after_ms:
            time.sleep(wait_after_ms / 1000)
        text = (
            "Computer action: press_element\n"
            f"Target: {target_label}\n"
            f"Pressed index: {pressed_index}\n"
            f"Method: {method}\n"
            f"Matched element: {json.dumps(element, ensure_ascii=False)}"
        )
        if args.get("screenshot", True):
            shot = take_screenshot({}, state_dir)
            text += f"\nScreenshot: {shot}"
        return ok(text, f"pressed element {target_label}")
    raise RuntimeError(f"no accessibility element matched {target_label}")


def focus_element(args, state_dir):
    ensure_macos("focus element")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    target_label = element_target_label(query, index)
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    _, elements = accessibility_snapshot_elements({
        "max_elements": max_elements,
        "include_values": True,
        "include_state": True,
    })

    matches = []
    for element in elements:
        if not element_matches_target(element, query, index, role, match_mode):
            continue
        matches.append(element)
        if len(matches) < occurrence:
            continue
        try:
            focused_index = int(element.get("index"))
        except (TypeError, ValueError) as exc:
            raise RuntimeError("matching accessibility element has no stable index") from exc
        method = focus_accessibility_element_by_index(focused_index)
        if wait_after_ms:
            time.sleep(wait_after_ms / 1000)
        text = (
            "Computer action: focus_element\n"
            f"Target: {target_label}\n"
            f"Focused index: {focused_index}\n"
            f"Method: {method}\n"
            f"Matched element: {json.dumps(element, ensure_ascii=False)}"
        )
        if args.get("screenshot", True):
            shot = take_screenshot({}, state_dir)
            text += f"\nScreenshot: {shot}"
        return ok(text, f"focused element {target_label}")
    raise RuntimeError(f"no accessibility element matched {target_label}")


def perform_element_action(args, state_dir):
    ensure_macos("perform element action")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    target_label = element_target_label(query, index)
    action = str(args.get("action") or "").strip()
    if action not in ALLOWED_ELEMENT_ACTIONS:
        raise RuntimeError("action is invalid")
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    _, elements = accessibility_snapshot_elements({
        "max_elements": max_elements,
        "include_values": True,
        "include_actions": True,
    })

    matches = []
    first_without_action = None
    for element in elements:
        if not element_matches_target(element, query, index, role, match_mode):
            continue
        element_actions = element.get("actions") if isinstance(element.get("actions"), list) else []
        if action not in element_actions:
            if first_without_action is None:
                first_without_action = element
            continue
        matches.append(element)
        if len(matches) < occurrence:
            continue
        try:
            matched_index = int(element.get("index"))
        except (TypeError, ValueError) as exc:
            raise RuntimeError("matching accessibility element has no stable index") from exc
        method = perform_accessibility_element_action_by_index(matched_index, action)
        if wait_after_ms:
            time.sleep(wait_after_ms / 1000)
        text = (
            "Computer action: perform_element_action\n"
            f"Target: {target_label}\n"
            f"Action: {action}\n"
            f"Matched index: {matched_index}\n"
            f"Method: {method}\n"
            f"Available actions: {', '.join(element_actions) if element_actions else '(none)'}\n"
            f"Matched element: {json.dumps(element, ensure_ascii=False)}"
        )
        if args.get("screenshot", True):
            shot = take_screenshot({}, state_dir)
            text += f"\nScreenshot: {shot}"
        return ok(text, f"performed {action} on element {target_label}")
    if first_without_action is not None:
        raise RuntimeError(f"matching accessibility element does not advertise {action}")
    raise RuntimeError(f"no accessibility element matched {target_label}")


def set_element_value(args, state_dir):
    ensure_macos("set element value")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    target_label = element_target_label(query, index)
    value = args.get("value")
    if not isinstance(value, str):
        raise RuntimeError("value is required")
    if len(value) > 10000:
        raise RuntimeError("value must be <= 10000 characters")
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    press_enter = bool_arg(args, "press_enter", False)
    _, elements = accessibility_snapshot_elements({"max_elements": max_elements, "include_values": True})

    matches = []
    for element in elements:
        if not element_matches_target(element, query, index, role, match_mode):
            continue
        matches.append(element)
        if len(matches) < occurrence:
            continue
        try:
            matched_index = int(element.get("index"))
        except (TypeError, ValueError) as exc:
            raise RuntimeError("matching accessibility element has no stable index") from exc
        method = set_accessibility_element_value_by_index(matched_index, value, press_enter=press_enter)
        if wait_after_ms:
            time.sleep(wait_after_ms / 1000)
        text = (
            "Computer action: set_element_value\n"
            f"Target: {target_label}\n"
            f"Matched index: {matched_index}\n"
            f"Value characters: {len(value)}\n"
            f"Press enter: {press_enter}\n"
            f"Method: {method}\n"
            f"Matched element: {json.dumps(element, ensure_ascii=False)}"
        )
        if args.get("screenshot", True):
            shot = take_screenshot({}, state_dir)
            text += f"\nScreenshot: {shot}"
        return ok(text, f"set element value {target_label}")
    raise RuntimeError(f"no accessibility element matched {target_label}")


def move_to_element(args, state_dir):
    ensure_macos("move to element")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    target_label = element_target_label(query, index)
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    wait_after_ms = int_arg(args, "wait_after_ms", 250, 0, 5000)
    offset_x = element_offset(args, "offset_x")
    offset_y = element_offset(args, "offset_y")
    _, elements = accessibility_snapshot_elements({"max_elements": max_elements, "include_values": True})

    matches = []
    unbounded = None
    for element in elements:
        if not element_matches_target(element, query, index, role, match_mode):
            continue
        point = element_point(element, offset_x, offset_y)
        if point is None:
            if unbounded is None:
                unbounded = element
            continue
        matches.append(element)
        if len(matches) < occurrence:
            continue
        x, y = point
        post_mouse(app_services(), 5, CGPoint(x, y), 0)
        if wait_after_ms:
            time.sleep(wait_after_ms / 1000)
        text = (
            "Computer action: move_to_element\n"
            f"Target: {target_label}\n"
            f"Moved: {x:g},{y:g}\n"
            f"Offset: {offset_x:g},{offset_y:g}\n"
            f"Matched element: {json.dumps(element, ensure_ascii=False)}"
        )
        if args.get("screenshot", True):
            shot = take_screenshot({}, state_dir)
            text += f"\nScreenshot: {shot}"
        return ok(text, f"moved to element {target_label}")
    if unbounded is not None:
        raise RuntimeError("matching accessibility element has no bounds")
    raise RuntimeError(f"no accessibility element matched {target_label}")


def type_element(args, state_dir):
    ensure_macos("type element")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    target_label = element_target_label(query, index)
    text_value = args.get("text")
    if not isinstance(text_value, str):
        raise RuntimeError("text is required")
    if len(text_value) > 10000:
        raise RuntimeError("text must be <= 10000 characters")
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    paste_settle_ms = int_arg(args, "paste_settle_ms", DEFAULT_PASTE_SETTLE_MS, 0, 2000)
    offset_x = element_offset(args, "offset_x")
    offset_y = element_offset(args, "offset_y")
    paste = bool_arg(args, "paste", True)
    clear = bool_arg(args, "clear", True)
    press_enter = bool_arg(args, "press_enter", False)
    _, elements = accessibility_snapshot_elements({"max_elements": max_elements, "include_values": True})

    matches = []
    unbounded = None
    for element in elements:
        if not element_matches_target(element, query, index, role, match_mode):
            continue
        point = element_point(element, offset_x, offset_y)
        if point is None:
            if unbounded is None:
                unbounded = element
            continue
        matches.append(element)
        if len(matches) < occurrence:
            continue
        x, y = point
        cg = app_services()
        point = CGPoint(x, y)
        _, down, up = button_events("left")
        post_mouse(cg, down, point, 0, click_count=1)
        post_mouse(cg, up, point, 0, click_count=1)
        time.sleep(0.08)
        if clear:
            clear_focused_text()
        if paste:
            paste_text_with_clipboard(text_value, settle_ms=paste_settle_ms, timeout=8)
        else:
            run_osascript(f'tell application "System Events" to keystroke "{quote_applescript(text_value)}"', timeout=20)
        if press_enter:
            press_enter_key()
        if wait_after_ms:
            time.sleep(wait_after_ms / 1000)
        text = (
            "Computer action: type_element\n"
            f"Target: {target_label}\n"
            f"Focused: {x:g},{y:g}\n"
            f"Offset: {offset_x:g},{offset_y:g}\n"
            f"Typed characters: {len(text_value)}\n"
            f"Paste settle ms: {paste_settle_ms}\n"
            f"Wait after ms: {wait_after_ms}\n"
            f"Matched element: {json.dumps(element, ensure_ascii=False)}"
        )
        if args.get("screenshot", True):
            shot = take_screenshot({}, state_dir)
            text += f"\nScreenshot: {shot}"
        return ok(text, f"typed into element {target_label}")
    if unbounded is not None:
        raise RuntimeError("matching accessibility element has no bounds")
    raise RuntimeError(f"no accessibility element matched {target_label}")


def assert_element(args, state_dir):
    ensure_macos("assert element")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    target_label = element_target_label(query, index)
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    value_contains = str(args.get("value_contains") or "").strip()
    if len(value_contains) > 200 or "\n" in value_contains or "\r" in value_contains:
        raise RuntimeError("value_contains is invalid")
    expected_exists = bool_arg(args, "exists", True)
    expected_state = state_expectations(args)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    meta, elements = accessibility_snapshot_elements({
        "max_elements": max_elements,
        "include_values": True,
        "include_state": bool({"focused", "selected"} & set(expected_state.keys())),
    })

    target_matches = [
        element
        for element in elements
        if element_matches_target(element, query, index, role, match_mode)
    ]
    if value_contains:
        needle = value_contains.lower()
        target_matches = [
            element
            for element in target_matches
            if needle in str(element.get("value") or "").lower()
        ]
    matches = [
        element
        for element in target_matches
        if element_matches_state_expectations(element, expected_state)
    ]

    frontmost = meta.get("frontmost_app") or "(unknown)"
    front_window = meta.get("front_window") or ""
    if bool(matches) == expected_exists:
        state = "exists" if expected_exists else "does not exist"
        text = (
            "Computer action: assert_element\n"
            f"Target: {target_label}\n"
            f"Expected exists: {str(expected_exists).lower()}\n"
            f"Expected state: {json.dumps(expected_state, ensure_ascii=False)}\n"
            f"State: {state}\n"
            f"Target match count: {len(target_matches)}\n"
            f"Match count: {len(matches)}\n"
            f"Frontmost app: {frontmost}\n"
            f"Front window: {front_window}\n"
            f"Matches: {json.dumps(matches[:10], ensure_ascii=False)}"
        )
        return ok(text, f"asserted element {target_label}")

    if expected_exists:
        state_clause = f" with state {json.dumps(expected_state, ensure_ascii=False)}" if expected_state else ""
        candidates = candidate_summary(elements)
        text = (
            "Computer action: assert_element\n"
            f"Assertion failed: expected accessibility element {target_label}{state_clause} to exist but found none.\n"
            f"Frontmost app: {frontmost}\n"
            f"Front window: {front_window}\n"
            f"Target matches: {json.dumps(target_matches[:10], ensure_ascii=False)}\n"
            f"Candidates: {json.dumps(candidates, ensure_ascii=False)}"
        )
        return err(text, "element assertion failed")

    text = (
        "Computer action: assert_element\n"
        f"Assertion failed: expected no accessibility element matching {target_label} but found {len(matches)}.\n"
        f"Frontmost app: {frontmost}\n"
        f"Front window: {front_window}\n"
        f"Matches: {json.dumps(matches[:10], ensure_ascii=False)}"
    )
    return err(text, "element assertion failed")


def wait_for_element(args, state_dir):
    ensure_macos("wait for element")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    target_label = element_target_label(query, index)
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    value_contains = str(args.get("value_contains") or "").strip()
    if len(value_contains) > 200 or "\n" in value_contains or "\r" in value_contains:
        raise RuntimeError("value_contains is invalid")
    expected_exists = bool_arg(args, "exists", True)
    expected_state = state_expectations(args)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    timeout_ms = int_arg(args, "timeout_ms", 5000, 0, 60000)
    interval_ms = int_arg(args, "interval_ms", 500, 100, 5000)

    started = time.monotonic()
    deadline = started + (timeout_ms / 1000)
    attempts = 0
    last_matches = []
    last_elements = []
    last_meta = {}
    while True:
        attempts += 1
        last_meta, elements = accessibility_snapshot_elements({
            "max_elements": max_elements,
            "include_values": True,
            "include_state": bool({"focused", "selected"} & set(expected_state.keys())),
        })
        last_elements = elements
        target_matches = [
            element
            for element in elements
            if element_matches_target(element, query, index, role, match_mode)
        ]
        if value_contains:
            needle = value_contains.lower()
            target_matches = [
                element
                for element in target_matches
                if needle in str(element.get("value") or "").lower()
            ]
        matches = [
            element
            for element in target_matches
            if element_matches_state_expectations(element, expected_state)
        ]
        last_matches = matches
        if bool(matches) == expected_exists:
            elapsed_ms = int((time.monotonic() - started) * 1000)
            state = "appeared" if expected_exists else "disappeared"
            text = (
                "Computer action: wait_for_element\n"
                f"Target: {target_label}\n"
                f"Expected exists: {str(expected_exists).lower()}\n"
                f"Expected state: {json.dumps(expected_state, ensure_ascii=False)}\n"
                f"State: {state}\n"
                f"Attempts: {attempts}\n"
                f"Elapsed ms: {elapsed_ms}\n"
                f"Target match count: {len(target_matches)}\n"
                f"Match count: {len(matches)}\n"
                f"Frontmost app: {last_meta.get('frontmost_app') or '(unknown)'}\n"
                f"Front window: {last_meta.get('front_window') or ''}\n"
                f"Matches: {json.dumps(matches[:10], ensure_ascii=False)}"
            )
            return ok(text, f"waited for element {target_label}")
        now = time.monotonic()
        if now >= deadline:
            break
        time.sleep(min(interval_ms / 1000, max(0, deadline - now)))

    if expected_exists:
        state_clause = f" with state {json.dumps(expected_state, ensure_ascii=False)}" if expected_state else ""
        text = (
            "Computer action: wait_for_element\n"
            f"Timed out after {timeout_ms}ms waiting for accessibility element {target_label}{state_clause} to exist.\n"
            f"Attempts: {attempts}\n"
            f"Frontmost app: {last_meta.get('frontmost_app') or '(unknown)'}\n"
            f"Front window: {last_meta.get('front_window') or ''}\n"
            f"Candidates: {json.dumps(candidate_summary(last_elements), ensure_ascii=False)}"
        )
        return err(text, "element wait timed out")

    text = (
        "Computer action: wait_for_element\n"
        f"Timed out after {timeout_ms}ms waiting for no accessibility element matching {target_label}.\n"
        f"Attempts: {attempts}\n"
        f"Frontmost app: {last_meta.get('frontmost_app') or '(unknown)'}\n"
        f"Front window: {last_meta.get('front_window') or ''}\n"
        f"Matches: {json.dumps(last_matches[:10], ensure_ascii=False)}"
    )
    return err(text, "element wait timed out")


def dragged_event(button):
    if button == 0:
        return 6
    if button == 1:
        return 7
    return 27


def perform_mouse_drag(cg, start_x, start_y, end_x, end_y, button, down, up, duration_ms, modifier_flags):
    drag_event = dragged_event(button)
    start = CGPoint(start_x, start_y)
    end = CGPoint(end_x, end_y)
    steps = 2 if duration_ms <= 0 else max(2, min(80, int(duration_ms / 16) + 1))
    delay = 0 if duration_ms <= 0 else duration_ms / max(steps - 1, 1) / 1000
    post_mouse(cg, 5, start, button, modifier_flags)
    post_mouse(cg, down, start, button, modifier_flags)
    for index in range(1, steps):
        ratio = index / (steps - 1)
        point = CGPoint(start_x + (end_x - start_x) * ratio, start_y + (end_y - start_y) * ratio)
        post_mouse(cg, drag_event, point, button, modifier_flags)
        if delay:
            time.sleep(delay)
    post_mouse(cg, up, end, button, modifier_flags)


def drag(args, state_dir):
    ensure_macos("drag")
    start_x = coord(args, "start_x")
    start_y = coord(args, "start_y")
    end_x = coord(args, "end_x")
    end_y = coord(args, "end_y")
    duration_ms = int_arg(args, "duration_ms", 500, 0, 10000)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    button, down, up = button_events(args.get("button"))
    modifier_flags, modifiers = mouse_modifier_flags(args.get("modifiers"))
    cg = app_services()
    perform_mouse_drag(cg, start_x, start_y, end_x, end_y, button, down, up, duration_ms, modifier_flags)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    text = (
        "Computer action: drag\n"
        f"From: {start_x:g},{start_y:g}\n"
        f"To: {end_x:g},{end_y:g}\n"
        f"Duration ms: {duration_ms}\n"
        f"Wait after ms: {wait_after_ms}\n"
        f"Modifiers: {', '.join(modifiers) if modifiers else '(none)'}"
    )
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, "dragged host mouse")


def drag_element_target(args, elements, match_mode):
    has_target_x = args.get("target_x") is not None
    has_target_y = args.get("target_y") is not None
    has_target_offset_x = args.get("target_offset_x") is not None and args.get("target_offset_x") != ""
    has_target_offset_y = args.get("target_offset_y") is not None and args.get("target_offset_y") != ""
    target_query = str(args.get("target_query") or "").strip()
    target_index = optional_element_index(args, "target_index")
    if has_target_x or has_target_y:
        if has_target_x != has_target_y:
            raise RuntimeError("target_x and target_y must be provided together")
        if target_query or target_index is not None:
            raise RuntimeError("target coordinates cannot be combined with target_query or target_index")
        if has_target_offset_x or has_target_offset_y:
            raise RuntimeError("target_offset_x/target_offset_y can only be used with target elements")
        target_x = coord(args, "target_x")
        target_y = coord(args, "target_y")
        return target_x, target_y, None, f"{target_x:g},{target_y:g}", 0, 0
    if not target_query and target_index is None:
        raise RuntimeError("target_x/target_y or target_query/target_index is required")
    if len(target_query) > 200 or "\n" in target_query or "\r" in target_query:
        raise RuntimeError("target_query is invalid")
    target_role = str(args.get("target_role") or "").strip()
    if len(target_role) > 80 or "\n" in target_role or "\r" in target_role:
        raise RuntimeError("target_role is invalid")
    target_match_mode = str(args.get("target_match_mode") or match_mode).strip().lower()
    if target_match_mode not in {"contains", "exact"}:
        raise RuntimeError("target_match_mode must be contains/exact")
    target_occurrence = int_arg(args, "target_occurrence", 1, 1, 20)
    target_offset_x = element_offset(args, "target_offset_x")
    target_offset_y = element_offset(args, "target_offset_y")
    target_label = element_target_label(target_query, target_index)
    seen = 0
    unbounded = None
    for element in elements:
        if not element_matches_target(element, target_query, target_index, target_role, target_match_mode):
            continue
        point = element_point(element, target_offset_x, target_offset_y)
        if point is None:
            if unbounded is None:
                unbounded = element
            continue
        seen += 1
        if seen >= target_occurrence:
            return point[0], point[1], element, target_label, target_offset_x, target_offset_y
    if unbounded is not None:
        raise RuntimeError(f"target accessibility element {target_label} has no bounds")
    raise RuntimeError(f"no target accessibility element matched {target_label}")


def drag_element(args, state_dir):
    ensure_macos("drag element")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    source_label = element_target_label(query, index)
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    duration_ms = int_arg(args, "duration_ms", 500, 0, 10000)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    offset_x = element_offset(args, "offset_x")
    offset_y = element_offset(args, "offset_y")
    button, down, up = button_events(args.get("button"))
    modifier_flags, modifiers = mouse_modifier_flags(args.get("modifiers"))
    _, elements = accessibility_snapshot_elements({"max_elements": max_elements, "include_values": True})
    target_x, target_y, target_element, target_label, target_offset_x, target_offset_y = drag_element_target(args, elements, match_mode)
    matches = []
    unbounded = None
    for element in elements:
        if not element_matches_target(element, query, index, role, match_mode):
            continue
        point = element_point(element, offset_x, offset_y)
        if point is None:
            if unbounded is None:
                unbounded = element
            continue
        matches.append(element)
        if len(matches) < occurrence:
            continue
        start_x, start_y = point
        cg = app_services()
        perform_mouse_drag(cg, start_x, start_y, target_x, target_y, button, down, up, duration_ms, modifier_flags)
        if wait_after_ms:
            time.sleep(wait_after_ms / 1000)
        text = (
            "Computer action: drag_element\n"
            f"Source: {source_label}\n"
            f"Target: {target_label}\n"
            f"From: {start_x:g},{start_y:g}\n"
            f"To: {target_x:g},{target_y:g}\n"
            f"Source offset: {offset_x:g},{offset_y:g}\n"
            f"Target offset: {target_offset_x:g},{target_offset_y:g}\n"
            f"Duration ms: {duration_ms}\n"
            f"Modifiers: {', '.join(modifiers) if modifiers else '(none)'}\n"
            f"Matched element: {json.dumps(element, ensure_ascii=False)}"
        )
        if target_element is not None:
            text += f"\nMatched target element: {json.dumps(target_element, ensure_ascii=False)}"
        if args.get("screenshot", True):
            shot = take_screenshot({}, state_dir)
            text += f"\nScreenshot: {shot}"
        return ok(text, f"dragged element {source_label}")
    if unbounded is not None:
        raise RuntimeError("matching accessibility element has no bounds")
    raise RuntimeError(f"no accessibility element matched {source_label}")


def key_script(key):
    parts = str(key or "").split("+")
    final_key = parts[-1]
    modifiers = []
    for part in parts[:-1]:
        mapped = MODIFIERS.get(part)
        if not mapped:
            raise RuntimeError("key modifier is not supported")
        modifiers.append(mapped)
    using = f" using {{{', '.join(modifiers)}}}" if modifiers else ""
    if final_key in KEY_CODES:
        return f'tell application "System Events" to key code {KEY_CODES[final_key]}{using}'
    if len(final_key) == 1 and final_key.isalnum():
        return f'tell application "System Events" to keystroke "{quote_applescript(final_key)}"{using}'
    raise RuntimeError("key is not supported")


def press_key(args, state_dir):
    ensure_macos("press key")
    key = str(args.get("key") or "").strip()
    output = run_osascript(key_script(key), timeout=8)
    wait_after_ms = int_arg(args, "wait_after_ms", 250, 0, 5000)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    text = f"Computer action: press_key\nPressed: {key}"
    text += f"\nWait after ms: {wait_after_ms}"
    if output:
        text += f"\nOutput: {output}"
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"pressed {key}")


def hotkey_script(key, raw_modifiers):
    final_key = str(key or "").strip()
    if not final_key:
        raise RuntimeError("key is required")
    if final_key in MODIFIERS:
        raise RuntimeError("key must be a non-modifier key")
    if not isinstance(raw_modifiers, list):
        raise RuntimeError("modifiers must be an array")
    if len(raw_modifiers) < 1 or len(raw_modifiers) > 4:
        raise RuntimeError("modifiers must contain 1 to 4 items")
    modifier_labels = []
    modifier_values = []
    seen = set()
    aliases = {"Meta": "Command", "Alt": "Option"}
    for raw_modifier in raw_modifiers:
        modifier = str(raw_modifier or "").strip()
        mapped = MODIFIERS.get(modifier)
        if mapped is None:
            raise RuntimeError("modifier is not supported")
        canonical = aliases.get(modifier, modifier)
        if canonical in seen:
            continue
        seen.add(canonical)
        modifier_labels.append(canonical)
        modifier_values.append(mapped)
    using = f" using {{{', '.join(modifier_values)}}}"
    if final_key in KEY_CODES:
        return f'tell application "System Events" to key code {KEY_CODES[final_key]}{using}', final_key, modifier_labels
    if len(final_key) == 1 and final_key.isalnum():
        return f'tell application "System Events" to keystroke "{quote_applescript(final_key)}"{using}', final_key, modifier_labels
    raise RuntimeError("key is not supported")


def hotkey(args, state_dir):
    ensure_macos("hotkey")
    script, key, modifiers = hotkey_script(args.get("key"), args.get("modifiers"))
    wait_after_ms = int_arg(args, "wait_after_ms", 250, 0, 5000)
    output = run_osascript(script, timeout=8)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    text = (
        "Computer action: hotkey\n"
        f"Key: {key}\n"
        f"Modifiers: {', '.join(modifiers)}\n"
        f"Wait after ms: {wait_after_ms}"
    )
    if output:
        text += f"\nOutput: {output}"
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"pressed hotkey {'+'.join([*modifiers, key])}")


def key_state_code(key):
    key = str(key or "").strip()
    canonical = KEY_STATE_ALIASES.get(key, key)
    if canonical in MODIFIER_KEY_CODES:
        return MODIFIER_KEY_CODES[canonical], canonical
    if canonical in KEY_CODES:
        return KEY_CODES[canonical], canonical
    if len(canonical) == 1 and canonical.isalnum():
        code = ALPHANUMERIC_KEY_CODES.get(canonical.lower())
        if code is not None:
            return code, canonical
    raise RuntimeError("key is not supported")


def key_state(args, state_dir, pressed):
    ensure_macos("key down" if pressed else "key up")
    raw_key = str(args.get("key") or "").strip()
    if not raw_key:
        raise RuntimeError("key is required")
    key_code, key = key_state_code(raw_key)
    wait_after_ms = int_arg(args, "wait_after_ms", 250, 0, 5000)
    post_keyboard(app_services(), key_code, pressed)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    action = "key_down" if pressed else "key_up"
    text = (
        f"Computer action: {action}\n"
        f"Key: {key}\n"
        f"Key code: {key_code}\n"
        f"Pressed: {str(pressed).lower()}\n"
        f"Wait after ms: {wait_after_ms}"
    )
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"{'pressed' if pressed else 'released'} key {key}")


def key_down(args, state_dir):
    return key_state(args, state_dir, True)


def key_up(args, state_dir):
    return key_state(args, state_dir, False)


def type_text(args, state_dir):
    ensure_macos("type text")
    text_value = args.get("text")
    if not isinstance(text_value, str):
        raise RuntimeError("text is required")
    if len(text_value) > 10000:
        raise RuntimeError("text must be <= 10000 characters")
    paste = bool_arg(args, "paste", True)
    paste_settle_ms = int_arg(args, "paste_settle_ms", DEFAULT_PASTE_SETTLE_MS, 0, 2000)
    clear = bool_arg(args, "clear", False)
    press_enter = bool_arg(args, "press_enter", False)
    if clear:
        clear_focused_text()
    if paste:
        paste_text_with_clipboard(text_value, settle_ms=paste_settle_ms, timeout=8)
    else:
        run_osascript(f'tell application "System Events" to keystroke "{quote_applescript(text_value)}"', timeout=20)
    if press_enter:
        press_enter_key()
    wait_after_ms = int_arg(args, "wait_after_ms", 250, 0, 5000)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    result = f"Computer action: type_text\nTyped characters: {len(text_value)}"
    result += f"\nPaste settle ms: {paste_settle_ms}"
    result += f"\nClear: {str(clear).lower()}"
    result += f"\nPress enter: {str(press_enter).lower()}"
    result += f"\nWait after ms: {wait_after_ms}"
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        result += f"\nScreenshot: {shot}"
    return ok(result, "typed into focused app")


def read_clipboard_text():
    ensure_macos("clipboard")
    completed = subprocess.run(["pbpaste"], capture_output=True, text=True, check=False, timeout=5)
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "").strip()
        raise RuntimeError(detail or f"pbpaste exited with code {completed.returncode}")
    return completed.stdout


def write_clipboard_text(text):
    ensure_macos("clipboard")
    try:
        subprocess.run(["pbcopy"], input=text, text=True, capture_output=True, check=True, timeout=5)
    except subprocess.CalledProcessError as exc:
        detail = (exc.stderr or exc.stdout or "").strip()
        raise RuntimeError(detail or f"pbcopy exited with code {exc.returncode}") from exc


def clip_text(text, max_chars):
    if len(text) <= max_chars:
        return text, False
    return text[:max_chars], True


def get_clipboard(args, state_dir):
    max_chars = int_arg(args, "max_chars", 4000, 1, 50000)
    text_value = read_clipboard_text()
    output, truncated = clip_text(text_value, max_chars)
    text = (
        "Computer action: get_clipboard\n"
        f"Clipboard characters: {len(text_value)}\n"
        f"Truncated: {str(truncated).lower()}\n"
        f"Text:\n{output}"
    )
    return ok(text, "read host clipboard")


def set_clipboard(args, state_dir):
    text_value = args.get("text")
    if not isinstance(text_value, str):
        raise RuntimeError("text is required")
    if len(text_value) > 100000:
        raise RuntimeError("text must be <= 100000 characters")
    write_clipboard_text(text_value)
    text = f"Computer action: set_clipboard\nClipboard characters: {len(text_value)}"
    return ok(text, "set host clipboard")


def copy_selection(args, state_dir):
    ensure_macos("copy selection")
    max_chars = int_arg(args, "max_chars", 4000, 1, 50000)
    wait_after_ms = int_arg(args, "wait_after_ms", 250, 0, 5000)
    restore_clipboard = bool_arg(args, "restore_clipboard", True)
    old = read_clipboard_text()
    restore_error = ""
    try:
        run_osascript('tell application "System Events" to keystroke "c" using command down', timeout=8)
        if wait_after_ms:
            time.sleep(wait_after_ms / 1000)
        selected = read_clipboard_text()
    finally:
        if restore_clipboard:
            try:
                write_clipboard_text(old)
            except Exception as exc:
                restore_error = str(exc)
    output, truncated = clip_text(selected, max_chars)
    text = (
        "Computer action: copy_selection\n"
        f"Copied characters: {len(selected)}\n"
        f"Restore clipboard: {str(restore_clipboard).lower()}\n"
        f"Truncated: {str(truncated).lower()}\n"
    )
    if restore_error:
        text += f"Restore warning: {restore_error}\n"
    text += f"Text:\n{output}"
    return ok(text, "copied selected text")


def clipboard_condition(args):
    expected = args.get("text")
    if not isinstance(expected, str):
        raise RuntimeError("text is required")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    if not expected and match_mode != "exact":
        raise RuntimeError("text is required unless match_mode is exact")
    expected_exists = bool_arg(args, "exists", True)
    max_chars = int_arg(args, "max_chars", 4000, 1, 50000)
    return expected, match_mode, expected_exists, max_chars


def clipboard_matches(value, expected, match_mode):
    if match_mode == "exact":
        return value == expected
    return expected in value


def clipboard_condition_text(action, value, output, truncated, found, expected, match_mode, expected_exists, extra=None):
    lines = [
        f"Computer action: {action}",
        f"Clipboard characters: {len(value)}",
        f"Output characters: {len(output)}",
        f"Truncated: {str(truncated).lower()}",
        f"Expected characters: {len(expected)}",
        f"Match mode: {match_mode}",
        f"Expected exists: {str(expected_exists).lower()}",
        f"Found: {str(found).lower()}",
    ]
    for key, raw_value in (extra or {}).items():
        lines.append(f"{key}: {raw_value}")
    lines.append(f"Text:\n{output}")
    return "\n".join(lines)


def assert_clipboard(args, state_dir):
    expected, match_mode, expected_exists, max_chars = clipboard_condition(args)
    value = read_clipboard_text()
    found = clipboard_matches(value, expected, match_mode)
    output, truncated = clip_text(value, max_chars)
    text = clipboard_condition_text(
        "assert_clipboard",
        value,
        output,
        truncated,
        found,
        expected,
        match_mode,
        expected_exists,
    )
    if found == expected_exists:
        return ok(text, "asserted host clipboard")
    return err(text, "clipboard assertion failed")


def wait_for_clipboard(args, state_dir):
    expected, match_mode, expected_exists, max_chars = clipboard_condition(args)
    timeout_ms = int_arg(args, "timeout_ms", 5000, 0, 60000)
    interval_ms = int_arg(args, "interval_ms", 250, 100, 5000)
    started = time.monotonic()
    deadline = started + (timeout_ms / 1000)
    attempts = 0
    value = ""
    found = False
    while True:
        attempts += 1
        value = read_clipboard_text()
        found = clipboard_matches(value, expected, match_mode)
        if found == expected_exists:
            elapsed_ms = int((time.monotonic() - started) * 1000)
            output, truncated = clip_text(value, max_chars)
            text = clipboard_condition_text(
                "wait_for_clipboard",
                value,
                output,
                truncated,
                found,
                expected,
                match_mode,
                expected_exists,
                {"Attempts": attempts, "Elapsed ms": elapsed_ms},
            )
            return ok(text, "waited for host clipboard")
        now = time.monotonic()
        if now >= deadline:
            break
        time.sleep(min(interval_ms / 1000, max(0, deadline - now)))

    output, truncated = clip_text(value, max_chars)
    text = clipboard_condition_text(
        "wait_for_clipboard",
        value,
        output,
        truncated,
        found,
        expected,
        match_mode,
        expected_exists,
        {"Attempts": attempts, "Timeout ms": timeout_ms, "Interval ms": interval_ms},
    )
    return err(text, "clipboard wait timed out")


def scroll(args, state_dir):
    ensure_macos("scroll")
    direction = str(args.get("direction") or "down").lower()
    amount = int_arg(args, "amount", 600, 1, 5000)
    target_x = target_y = None
    if args.get("x") is not None or args.get("y") is not None:
        if args.get("x") is None or args.get("y") is None:
            raise RuntimeError("x and y must be provided together")
        target_x = coord(args, "x")
        target_y = coord(args, "y")
    wait_after_ms = int_arg(args, "wait_after_ms", 250, 0, 5000)
    modifier_flags, modifiers = mouse_modifier_flags(args.get("modifiers"))
    dx = dy = 0
    if direction == "down":
        dy = -amount
    elif direction == "up":
        dy = amount
    elif direction == "left":
        dx = amount
    elif direction == "right":
        dx = -amount
    else:
        raise RuntimeError("direction must be down/up/left/right")
    cg = app_services()
    if target_x is not None and target_y is not None:
        post_mouse(cg, 5, CGPoint(target_x, target_y), 0)
    cg.CGEventCreateScrollWheelEvent.restype = ctypes.c_void_p
    cg.CGEventSetFlags.argtypes = [ctypes.c_void_p, ctypes.c_uint64]
    cg.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
    event = cg.CGEventCreateScrollWheelEvent(None, 0, 2, int(dy), int(dx))
    if not event:
        raise RuntimeError("failed to create scroll event")
    try:
        if modifier_flags:
            cg.CGEventSetFlags(event, modifier_flags)
        cg.CGEventPost(0, event)
    finally:
        cf_release(event)
    text = f"Computer action: scroll\nDirection: {direction}\nAmount: {amount}"
    if target_x is not None and target_y is not None:
        text += f"\nTarget: {target_x:g},{target_y:g}"
    if modifiers:
        text += f"\nModifiers: {', '.join(modifiers)}"
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"scrolled {direction}")


def scroll_element(args, state_dir):
    ensure_macos("scroll element")
    query = str(args.get("query") or "").strip()
    if len(query) > 200 or "\n" in query or "\r" in query:
        raise RuntimeError("query is invalid")
    index = optional_element_index(args)
    if not query and index is None:
        raise RuntimeError("query or index is required")
    target_label = element_target_label(query, index)
    role = str(args.get("role") or "").strip()
    if len(role) > 80 or "\n" in role or "\r" in role:
        raise RuntimeError("role is invalid")
    match_mode = str(args.get("match_mode") or "contains").strip().lower()
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_elements = int_arg(args, "max_elements", 120, 1, 500)
    offset_x = element_offset(args, "offset_x")
    offset_y = element_offset(args, "offset_y")
    direction = str(args.get("direction") or "down").lower()
    if direction not in {"down", "up", "left", "right"}:
        raise RuntimeError("direction must be down/up/left/right")
    amount = int_arg(args, "amount", 600, 1, 5000)
    wait_after_ms = int_arg(args, "wait_after_ms", 250, 0, 5000)
    mouse_modifier_flags(args.get("modifiers"))
    _, elements = accessibility_snapshot_elements({"max_elements": max_elements, "include_values": True})

    matches = []
    unbounded = None
    for element in elements:
        if not element_matches_target(element, query, index, role, match_mode):
            continue
        point = element_point(element, offset_x, offset_y)
        if point is None:
            if unbounded is None:
                unbounded = element
            continue
        matches.append(element)
        if len(matches) < occurrence:
            continue
        x, y = point
        scroll_args = dict(args)
        scroll_args["direction"] = direction
        scroll_args["amount"] = amount
        scroll_args["wait_after_ms"] = wait_after_ms
        scroll_args["x"] = x
        scroll_args["y"] = y
        result = scroll(scroll_args, state_dir)
        result["text"] = (
            "Computer action: scroll_element\n"
            f"Target element: {target_label}\n"
            f"Point: {x:g},{y:g}\n"
            f"Offset: {offset_x:g},{offset_y:g}\n"
            f"Matched element: {json.dumps(element, ensure_ascii=False)}\n"
            + "\n".join(str(result.get("text") or "").splitlines()[1:])
        )
        result["display"] = f"scrolled element {target_label}"
        return result
    if unbounded is not None:
        raise RuntimeError("matching accessibility element has no bounds")
    raise RuntimeError(f"no accessibility element matched {target_label}")


def list_apps(args, state_dir):
    ensure_macos("list apps")
    include_background = bool_arg(args, "include_background", False)
    limit = int_arg(args, "max_apps", 100, 1, 500)
    include_background_script = "true" if include_background else "false"
    script = f"""
set previousDelimiters to AppleScript's text item delimiters
set AppleScript's text item delimiters to linefeed
set rows to {{}}
set includeBackground to {include_background_script}
set maxRows to {limit}
tell application "System Events"
    set frontName to ""
    try
        set frontName to name of first application process whose frontmost is true
    end try
    repeat with p in application processes
        if (count of rows) >= maxRows then exit repeat
        set procVisible to false
        set procFrontmost to false
        set appName to ""
        set bundleId to ""
        try
            set procVisible to visible of p
        end try
        if includeBackground or procVisible then
            try
                set appName to name of p
            end try
            try
                set bundleId to bundle identifier of p
            end try
            try
                set procFrontmost to frontmost of p
            end try
            if appName is not "" then
                set end of rows to appName & tab & bundleId & tab & (procVisible as text) & tab & (procFrontmost as text) & tab & frontName
            end if
        end if
    end repeat
end tell
set outputText to rows as text
set AppleScript's text item delimiters to previousDelimiters
return outputText
"""
    output = run_osascript(script, timeout=12)
    apps = []
    frontmost = ""
    for line in output.splitlines():
        parts = line.split("\t")
        if len(parts) < 4:
            continue
        if len(parts) >= 5 and parts[4]:
            frontmost = parts[4]
        apps.append({
            "index": len(apps) + 1,
            "name": parts[0],
            "bundle_identifier": parts[1],
            "visible": parts[2].strip().lower() in {"true", "1", "yes"},
            "frontmost": parts[3].strip().lower() in {"true", "1", "yes"},
        })
        if len(apps) >= limit:
            break
    if not frontmost:
        frontmost = next((app["name"] for app in apps if app.get("frontmost")), "")
    text = "Computer action: list_apps"
    if frontmost:
        text += f"\nFrontmost app: {frontmost}"
    text += f"\nApps: {json.dumps(apps, ensure_ascii=False)}"
    return ok(text, f"listed {len(apps)} apps")


def apps_from_list_result(listed):
    text = str(listed.get("text") or "")
    frontmost = ""
    for line in text.splitlines():
        if line.startswith("Frontmost app: "):
            frontmost = line.split(": ", 1)[1].strip()
            break
    marker = "Apps: "
    apps = []
    if marker in text:
        try:
            apps = json.loads(text.split(marker, 1)[1].splitlines()[0])
        except Exception:
            apps = []
    return frontmost, apps


def app_query_parts(args):
    app_query = str(args.get("app") or "").strip()
    bundle_query = str(args.get("bundle_id") or "").strip()
    if app_query.startswith("id:") and not bundle_query:
        bundle_query = app_query[3:].strip()
        app_query = ""
    elif app_query.startswith("app:"):
        app_query = app_query[4:].strip()
    if not app_query and not bundle_query:
        raise RuntimeError("app or bundle_id is required")
    if app_query and (len(app_query) > 120 or "\n" in app_query or "\r" in app_query):
        raise RuntimeError("app is invalid")
    if bundle_query and (len(bundle_query) > 200 or "\n" in bundle_query or "\r" in bundle_query):
        raise RuntimeError("bundle_id is invalid")
    return app_query, bundle_query


def app_matches(app, app_query, bundle_query, match_mode):
    if app_query and not window_text_matches(app.get("name"), app_query, match_mode):
        return False
    if bundle_query and str(app.get("bundle_identifier") or "").casefold() != bundle_query.casefold():
        return False
    return True


def app_condition_state(args, state_dir):
    app_query, bundle_query = app_query_parts(args)
    match_mode = str(args.get("match_mode") or "contains")
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    expected_running = bool_arg(args, "running", True)
    visible_expected = args.get("visible")
    frontmost_expected = args.get("frontmost")
    max_apps = int_arg(args, "max_apps", 100, 1, 500)
    frontmost, apps = apps_from_list_result(
        list_apps({"include_background": True, "max_apps": max_apps}, state_dir)
    )
    matches = [
        app for app in apps
        if isinstance(app, dict) and app_matches(app, app_query, bundle_query, match_mode)
    ]
    matched = matches[0] if matches else None
    found = matched is not None
    visible_actual = bool(matched.get("visible")) if matched else None
    frontmost_actual = bool(matched.get("frontmost")) if matched else None
    visible_ok = True
    frontmost_ok = True
    if found and isinstance(visible_expected, bool):
        visible_ok = visible_actual == visible_expected
    if found and isinstance(frontmost_expected, bool):
        frontmost_ok = frontmost_actual == frontmost_expected
    condition_met = found == expected_running
    if expected_running and found:
        condition_met = condition_met and visible_ok and frontmost_ok
    return {
        "found": found,
        "condition_met": condition_met,
        "expected_running": expected_running,
        "app_query": app_query,
        "bundle_id_query": bundle_query,
        "match_mode": match_mode,
        "match_count": len(matches),
        "matched_app": matched or {},
        "frontmost_app": frontmost,
        "visible_expected": visible_expected if isinstance(visible_expected, bool) else None,
        "visible_actual": visible_actual,
        "frontmost_expected": frontmost_expected if isinstance(frontmost_expected, bool) else None,
        "frontmost_actual": frontmost_actual,
        "candidate_apps": apps[:20],
    }


def assert_app(args, state_dir):
    ensure_macos("assert app")
    state = app_condition_state(args, state_dir)
    text = "Computer action: assert_app\n" + json.dumps(state, ensure_ascii=False, indent=2)
    if state.get("condition_met"):
        return ok(text, "app condition matched")
    raise RuntimeError("expected app condition was not met\n" + text)


def wait_for_app(args, state_dir):
    ensure_macos("wait for app")
    timeout_ms = int_arg(args, "timeout_ms", 5000, 0, 60000)
    interval_ms = int_arg(args, "interval_ms", 500, 100, 5000)
    started = time.monotonic()
    deadline = started + (timeout_ms / 1000)
    attempts = 0
    last_state = {}
    while True:
        attempts += 1
        state = app_condition_state(args, state_dir)
        state["attempts"] = attempts
        state["elapsed_ms"] = int((time.monotonic() - started) * 1000)
        last_state = state
        if state.get("condition_met"):
            text = "Computer action: wait_for_app\n" + json.dumps(state, ensure_ascii=False, indent=2)
            return ok(text, "app condition matched")
        now = time.monotonic()
        if now >= deadline:
            break
        time.sleep(min(interval_ms / 1000, max(0, deadline - now)))
    raise RuntimeError(
        f"app condition timed out after {timeout_ms}ms\n"
        + json.dumps(last_state, ensure_ascii=False)
    )


def list_windows(args, state_dir):
    ensure_macos("list windows")
    limit = int_arg(args, "max_windows", 30, 1, 200)
    script = """
set previousDelimiters to AppleScript's text item delimiters
set AppleScript's text item delimiters to linefeed
set rows to {}
tell application "System Events"
    set frontName to ""
    try
        set frontName to name of first application process whose frontmost is true
    end try
    repeat with p in (application processes whose visible is true)
        set appName to name of p
        set appWindowIndex to 0
        repeat with w in windows of p
            try
                set appWindowIndex to appWindowIndex + 1
                set windowName to name of w
                set windowPosition to position of w
                set windowSize to size of w
                set minimizedText to ""
                set fullScreenText to ""
                try
                    set minimizedText to value of attribute "AXMinimized" of w as text
                end try
                try
                    set fullScreenText to value of attribute "AXFullScreen" of w as text
                end try
                set end of rows to appName & tab & windowName & tab & (appWindowIndex as text) & tab & ((item 1 of windowPosition) as text) & "," & ((item 2 of windowPosition) as text) & "," & ((item 1 of windowSize) as text) & "," & ((item 2 of windowSize) as text) & tab & frontName & tab & minimizedText & tab & fullScreenText
            end try
        end repeat
    end repeat
end tell
set outputText to rows as text
set AppleScript's text item delimiters to previousDelimiters
return outputText
"""
    output = run_osascript(script, timeout=12)
    windows = []
    frontmost = ""
    for line in output.splitlines():
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        app_window_index = None
        minimized_text = ""
        full_screen_text = ""
        if len(parts) >= 5:
            try:
                app_window_index = int(float(parts[2]))
            except (TypeError, ValueError):
                app_window_index = None
            bounds = parts[3]
            if parts[4]:
                frontmost = parts[4]
            if len(parts) >= 6:
                minimized_text = parts[5]
            if len(parts) >= 7:
                full_screen_text = parts[6]
        else:
            bounds = parts[2]
            if len(parts) >= 4 and parts[3]:
                frontmost = parts[3]
        item = {"index": len(windows) + 1, "app": parts[0], "title": parts[1]}
        if app_window_index is not None:
            item["app_window_index"] = app_window_index
        try:
            x_text, y_text, width_text, height_text = bounds.split(",", 3)
            item.update({
                "x": int(float(x_text)),
                "y": int(float(y_text)),
                "width": int(float(width_text)),
                "height": int(float(height_text)),
            })
        except (TypeError, ValueError):
            item["bounds"] = bounds
        minimized = parse_optional_bool_text(minimized_text)
        if minimized is not None:
            item["minimized"] = minimized
        full_screen = parse_optional_bool_text(full_screen_text)
        if full_screen is not None:
            item["full_screen"] = full_screen
        windows.append(item)
        if len(windows) >= limit:
            break
    text = "Computer action: list_windows"
    if frontmost:
        text += f"\nFrontmost app: {frontmost}"
    text += f"\nWindows: {json.dumps(windows, ensure_ascii=False)}"
    return ok(text, f"listed {len(windows)} windows")


def parse_optional_bool_text(value):
    normalized = str(value or "").strip().casefold()
    if normalized in {"true", "yes", "1"}:
        return True
    if normalized in {"false", "no", "0"}:
        return False
    return None


def window_text_matches(value, expected, match_mode):
    haystack = re.sub(r"\s+", " ", str(value or "")).strip().casefold()
    needle = re.sub(r"\s+", " ", str(expected or "")).strip().casefold()
    if match_mode == "exact":
        return haystack == needle
    return needle in haystack


def find_window_match(args, state_dir):
    index = optional_int_arg(args, "index", 1, 500)
    app = str(args.get("app") or "").strip()
    title = str(args.get("title") or "").strip()
    if index is None and not app and not title:
        raise RuntimeError("app, title, or index is required")
    if len(app) > 120 or "\n" in app or "\r" in app:
        raise RuntimeError("app is invalid")
    if len(title) > 300 or "\n" in title or "\r" in title:
        raise RuntimeError("title is invalid")
    match_mode = str(args.get("match_mode") or "contains")
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_windows = int_arg(args, "max_windows", 50, 1, 200)

    listed = list_windows({"max_windows": max_windows}, state_dir)
    windows_text = listed.get("text") or ""
    marker = "Windows: "
    windows = []
    if marker in windows_text:
        try:
            windows = json.loads(windows_text.split(marker, 1)[1].splitlines()[0])
        except Exception:
            windows = []
    matches = []
    for window in windows:
        if not isinstance(window, dict):
            continue
        if index is not None and int(window.get("index") or 0) != index:
            continue
        if app and not window_text_matches(window.get("app"), app, match_mode):
            continue
        if title and not window_text_matches(window.get("title"), title, match_mode):
            continue
        matches.append(window)
    if len(matches) < occurrence:
        raise RuntimeError("no visible window matched app/title/index")
    return matches, matches[occurrence - 1]


def windows_from_list_result(listed):
    text = listed.get("text") or ""
    frontmost = ""
    for line in text.splitlines():
        if line.startswith("Frontmost app: "):
            frontmost = line.split(": ", 1)[1]
            break
    marker = "Windows: "
    windows = []
    if marker in text:
        try:
            windows = json.loads(text.split(marker, 1)[1].splitlines()[0])
        except Exception:
            windows = []
    return frontmost, windows


def window_bounds_state(window, args):
    keys = ("x", "y", "width", "height")
    if not any(args.get(key) is not None and args.get(key) != "" for key in keys):
        return True, {}
    if any(args.get(key) is None or args.get(key) == "" for key in keys):
        raise RuntimeError("x, y, width, and height must be provided together")
    expected = {
        "x": int(round(float_arg(args, "x", MIN_SCREEN_COORDINATE, MAX_SCREEN_COORDINATE))),
        "y": int(round(float_arg(args, "y", MIN_SCREEN_COORDINATE, MAX_SCREEN_COORDINATE))),
        "width": int_arg(args, "width", 0, 1, 100000),
        "height": int_arg(args, "height", 0, 1, 100000),
    }
    tolerance = int_arg(args, "tolerance", 8, 0, 500)
    actual = {}
    for key in keys:
        try:
            actual[key] = int(float(window.get(key)))
        except (TypeError, ValueError):
            actual[key] = None
    deltas = {
        key: None if actual[key] is None else abs(int(actual[key]) - int(expected[key]))
        for key in keys
    }
    matched = all(delta is not None and delta <= tolerance for delta in deltas.values())
    return matched, {"expected": expected, "actual": actual, "tolerance": tolerance, "deltas": deltas}


def window_condition_state(args, state_dir):
    expected_exists = bool_arg(args, "exists", True)
    index = optional_int_arg(args, "index", 1, 500)
    app = str(args.get("app") or "").strip()
    title = str(args.get("title") or "").strip()
    if index is None and not app and not title:
        raise RuntimeError("app, title, or index is required")
    match_mode = str(args.get("match_mode") or "contains")
    if match_mode not in {"contains", "exact"}:
        raise RuntimeError("match_mode must be contains/exact")
    occurrence = int_arg(args, "occurrence", 1, 1, 20)
    max_windows = int_arg(args, "max_windows", 50, 1, 200)
    frontmost, windows = windows_from_list_result(list_windows({"max_windows": max_windows}, state_dir))
    matches = []
    for window in windows:
        if not isinstance(window, dict):
            continue
        if index is not None and int(window.get("index") or 0) != index:
            continue
        if app and not window_text_matches(window.get("app"), app, match_mode):
            continue
        if title and not window_text_matches(window.get("title"), title, match_mode):
            continue
        matches.append(window)
    matched = matches[occurrence - 1] if len(matches) >= occurrence else None
    found = matched is not None
    focused_expected = args.get("focused")
    focused_actual = None
    focused_ok = True
    if found and isinstance(focused_expected, bool):
        focused_actual = str(matched.get("app") or "") == frontmost
        focused_ok = focused_actual == focused_expected
    minimized_expected = optional_bool_arg(args, "minimized")
    minimized_actual = None
    minimized_ok = True
    if found and isinstance(minimized_expected, bool):
        minimized_actual = matched.get("minimized") if isinstance(matched.get("minimized"), bool) else None
        minimized_ok = minimized_actual == minimized_expected
    full_screen_expected = optional_bool_arg(args, "full_screen")
    full_screen_actual = None
    full_screen_ok = True
    if found and isinstance(full_screen_expected, bool):
        full_screen_actual = matched.get("full_screen") if isinstance(matched.get("full_screen"), bool) else None
        full_screen_ok = full_screen_actual == full_screen_expected
    bounds_ok = True
    bounds = {}
    if found:
        bounds_ok, bounds = window_bounds_state(matched, args)
    condition_met = found == expected_exists
    if expected_exists and found:
        condition_met = condition_met and focused_ok and minimized_ok and full_screen_ok and bounds_ok
    return {
        "found": found,
        "condition_met": condition_met,
        "expected_exists": expected_exists,
        "match_count": len(matches),
        "matched_window": matched or {},
        "frontmost_app": frontmost,
        "focused_expected": focused_expected if isinstance(focused_expected, bool) else None,
        "focused_actual": focused_actual,
        "minimized_expected": minimized_expected,
        "minimized_actual": minimized_actual,
        "full_screen_expected": full_screen_expected,
        "full_screen_actual": full_screen_actual,
        "bounds": bounds,
        "candidate_windows": windows[:20],
    }


def assert_window(args, state_dir):
    ensure_macos("assert window")
    state = window_condition_state(args, state_dir)
    text = "Computer action: assert_window\nWindow condition: " + json.dumps(state, ensure_ascii=False)
    if not state.get("condition_met"):
        raise RuntimeError("expected window condition was not met\n" + text)
    return ok(text, "window condition matched")


def wait_for_window(args, state_dir):
    ensure_macos("wait for window")
    timeout_ms = int_arg(args, "timeout_ms", 5000, 0, 60000)
    interval_ms = int_arg(args, "interval_ms", 500, 100, 5000)
    started = time.monotonic()
    deadline = started + (timeout_ms / 1000)
    attempts = 0
    last_state = {}
    while True:
        attempts += 1
        state = window_condition_state(args, state_dir)
        state["attempts"] = attempts
        state["elapsed_ms"] = int((time.monotonic() - started) * 1000)
        last_state = state
        if state.get("condition_met"):
            text = "Computer action: wait_for_window\nWindow condition: " + json.dumps(state, ensure_ascii=False)
            return ok(text, "window condition matched")
        now = time.monotonic()
        if now >= deadline:
            break
        time.sleep(min(interval_ms / 1000, max(0, deadline - now)))
    raise RuntimeError(
        f"window condition timed out after {timeout_ms}ms\n"
        + json.dumps(last_state, ensure_ascii=False)
    )


def focus_window(args, state_dir):
    ensure_macos("focus window")
    matches, matched = find_window_match(args, state_dir)
    app_name = str(matched.get("app") or "").strip()
    if not app_name:
        raise RuntimeError("matched window did not include an app name")
    app_window_index = int(matched.get("app_window_index") or 1)
    script = f'''
tell application "System Events"
    tell application process "{quote_applescript(app_name)}"
        set frontmost to true
        try
            perform action "AXRaise" of window {app_window_index}
        end try
    end tell
end tell
'''
    output = run_osascript(script, timeout=12)
    text = (
        "Computer action: focus_window"
        f"\nFocused window: {json.dumps(matched, ensure_ascii=False)}"
        f"\nMatch count: {len(matches)}"
    )
    if output:
        text += f"\nOutput: {output}"
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"focused {app_name}")


def set_window_bounds(args, state_dir):
    ensure_macos("set window bounds")
    target_x = int(round(float_arg(args, "x", MIN_SCREEN_COORDINATE, MAX_SCREEN_COORDINATE)))
    target_y = int(round(float_arg(args, "y", MIN_SCREEN_COORDINATE, MAX_SCREEN_COORDINATE)))
    width = int_arg(args, "width", 0, 1, 100000)
    height = int_arg(args, "height", 0, 1, 100000)
    matches, matched = find_window_match(args, state_dir)
    app_name = str(matched.get("app") or "").strip()
    if not app_name:
        raise RuntimeError("matched window did not include an app name")
    app_window_index = int(matched.get("app_window_index") or 1)
    script = f'''
tell application "System Events"
    tell application process "{quote_applescript(app_name)}"
        set frontmost to true
        set targetWindow to window {app_window_index}
        try
            perform action "AXRaise" of targetWindow
        end try
        set position of targetWindow to {{{target_x}, {target_y}}}
        set size of targetWindow to {{{width}, {height}}}
    end tell
end tell
'''
    output = run_osascript(script, timeout=12)
    text = (
        "Computer action: set_window_bounds"
        f"\nWindow: {json.dumps(matched, ensure_ascii=False)}"
        f"\nRequested bounds: {target_x},{target_y},{width},{height}"
        f"\nMatch count: {len(matches)}"
    )
    if output:
        text += f"\nOutput: {output}"
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"set {app_name} window bounds")


def perform_window_action(args, state_dir):
    ensure_macos("perform window action")
    action = str(args.get("action") or "").strip()
    if action not in ALLOWED_WINDOW_ACTIONS:
        raise RuntimeError("action is invalid")
    matches, matched = find_window_match(args, state_dir)
    app_name = str(matched.get("app") or "").strip()
    if not app_name:
        raise RuntimeError("matched window did not include an app name")
    app_window_index = int(matched.get("app_window_index") or 1)
    script = f'''
set requestedAction to "{action}"
tell application "System Events"
    tell application process "{quote_applescript(app_name)}"
        set frontmost to true
        set targetWindow to window {app_window_index}
        if requestedAction is "raise" then
            perform action "AXRaise" of targetWindow
            return "AXRaise"
        else if requestedAction is "minimize" then
            try
                set value of attribute "AXMinimized" of targetWindow to true
                return "AXMinimized=true"
            on error minimizeError
                try
                    perform action "AXPress" of button 2 of targetWindow
                    return "AXMinimizeButton"
                on error buttonError
                    error minimizeError & " | minimize button failed: " & buttonError
                end try
            end try
        else if requestedAction is "zoom" then
            try
                perform action "AXZoomWindow" of targetWindow
                return "AXZoomWindow"
            on error zoomError
                try
                    perform action "AXPress" of button 3 of targetWindow
                    return "AXZoomButton"
                on error buttonError
                    error zoomError & " | zoom button failed: " & buttonError
                end try
            end try
        else if requestedAction is "enter_full_screen" then
            set value of attribute "AXFullScreen" of targetWindow to true
            return "AXFullScreen=true"
        else if requestedAction is "exit_full_screen" then
            set value of attribute "AXFullScreen" of targetWindow to false
            return "AXFullScreen=false"
        else if requestedAction is "toggle_full_screen" then
            set isFullScreen to false
            try
                set isFullScreen to value of attribute "AXFullScreen" of targetWindow as boolean
            end try
            if isFullScreen then
                set value of attribute "AXFullScreen" of targetWindow to false
                return "AXFullScreen=false"
            else
                set value of attribute "AXFullScreen" of targetWindow to true
                return "AXFullScreen=true"
            end if
        end if
    end tell
end tell
error "unsupported window action"
'''
    method = run_osascript(script, timeout=12).strip() or action
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    text = (
        "Computer action: perform_window_action"
        f"\nAction: {action}"
        f"\nMethod: {method}"
        f"\nWindow: {json.dumps(matched, ensure_ascii=False)}"
        f"\nMatch count: {len(matches)}"
        f"\nWait after ms: {wait_after_ms}"
    )
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"performed {action} on {app_name} window")


def close_window(args, state_dir):
    ensure_macos("close window")
    matches, matched = find_window_match(args, state_dir)
    app_name = str(matched.get("app") or "").strip()
    if not app_name:
        raise RuntimeError("matched window did not include an app name")
    app_window_index = int(matched.get("app_window_index") or 1)
    script = f'''
tell application "System Events"
    tell application process "{quote_applescript(app_name)}"
        set frontmost to true
        set targetWindow to window {app_window_index}
        try
            perform action "AXRaise" of targetWindow
        end try
        try
            perform action "AXClose" of targetWindow
        on error
            perform action "AXPress" of button 1 of targetWindow
        end try
    end tell
end tell
'''
    output = run_osascript(script, timeout=12)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    text = (
        "Computer action: close_window"
        f"\nClosed window: {json.dumps(matched, ensure_ascii=False)}"
        f"\nMatch count: {len(matches)}"
        f"\nWait after ms: {wait_after_ms}"
    )
    if output:
        text += f"\nOutput: {output}"
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"closed {app_name} window")


def computer_app_name(args):
    app_name = str(args.get("app") or "").strip()
    if not app_name:
        raise RuntimeError("app is required")
    if len(app_name) > 120 or "\n" in app_name or "\r" in app_name:
        raise RuntimeError("app is invalid")
    return app_name


def menu_path_arg(args):
    raw_path = args.get("menu_path")
    if not isinstance(raw_path, list):
        raise RuntimeError("menu_path must be an array")
    if len(raw_path) < 2 or len(raw_path) > 8:
        raise RuntimeError("menu_path must include 2 to 8 items")
    path = []
    for item in raw_path:
        label = str(item or "").strip()
        if not label or len(label) > 120 or "\n" in label or "\r" in label:
            raise RuntimeError("menu_path contains an invalid item")
        path.append(label)
    return path


def application_process_setup_script(app_name):
    quoted = quote_applescript(app_name)
    return f'''
set targetName to "{quoted}"
if targetName starts with "app:" then set targetName to text 5 thru -1 of targetName
if targetName starts with "id:" then
    set bundleId to text 4 thru -1 of targetName
    set targetProcess to first application process whose bundle identifier is bundleId
else
    set targetProcess to application process targetName
end if
'''


def menu_item_reference(menu_path):
    top = quote_applescript(menu_path[0])
    container = f'menu "{top}" of menu bar item "{top}" of menu bar 1'
    for item in menu_path[1:-1]:
        container = f'menu 1 of menu item "{quote_applescript(item)}" of {container}'
    return f'menu item "{quote_applescript(menu_path[-1])}" of {container}'


def optional_menu_label(args, key):
    raw_value = args.get(key)
    if raw_value is None or raw_value == "":
        return ""
    label = str(raw_value or "").strip()
    if not label or len(label) > 120 or "\n" in label or "\r" in label:
        raise RuntimeError(f"{key} is invalid")
    return label


def parse_menu_lines(output):
    items = []
    for line in str(output or "").splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        if len(parts) < 5:
            continue
        try:
            depth = int(parts[0])
        except ValueError:
            depth = 0
        path = [part.strip() for part in parts[1].split(" > ") if part.strip()]
        if not path:
            continue
        items.append({
            "index": len(items) + 1,
            "depth": depth,
            "path": path,
            "path_text": " > ".join(path),
            "label": parts[2],
            "role": parts[3],
            "enabled": parts[4].strip().lower() in {"true", "1", "yes"},
        })
    return items


def list_menus(args, state_dir):
    ensure_macos("list menus")
    app_name = computer_app_name(args)
    top_menu = optional_menu_label(args, "top_menu")
    max_items = int_arg(args, "max_items", 120, 1, 500)
    script = f'''
on joinLines(items)
    set oldDelimiters to AppleScript's text item delimiters
    set AppleScript's text item delimiters to linefeed
    set joinedText to items as text
    set AppleScript's text item delimiters to oldDelimiters
    return joinedText
end joinLines

on appendMenuItems(theMenu, parentPath, itemDepth, itemLimit)
    set resultLines to {{}}
    if itemLimit <= 0 then return resultLines
    try
        repeat with menuItemRef in menu items of theMenu
            if (count of resultLines) >= itemLimit then exit repeat
            set itemTitle to ""
            try
                set itemTitle to name of menuItemRef as text
            end try
            if itemTitle is not "" then
                set itemRole to ""
                set itemEnabled to true
                try
                    set itemRole to role of menuItemRef as text
                end try
                try
                    set itemEnabled to enabled of menuItemRef
                end try
                set currentPath to parentPath & " > " & itemTitle
                set end of resultLines to (itemDepth as text) & tab & currentPath & tab & itemTitle & tab & itemRole & tab & (itemEnabled as text)
                try
                    set childLines to my appendMenuItems(menu 1 of menuItemRef, currentPath, itemDepth + 1, itemLimit - (count of resultLines))
                    repeat with childLine in childLines
                        set end of resultLines to contents of childLine
                        if (count of resultLines) >= itemLimit then exit repeat
                    end repeat
                end try
            end if
        end repeat
    end try
    return resultLines
end appendMenuItems

tell application "System Events"
    {application_process_setup_script(app_name)}
    set topFilter to "{quote_applescript(top_menu)}"
    set itemLimit to {max_items}
    set allLines to {{}}
    tell targetProcess
        set frontmost to true
        repeat with barItem in menu bar items of menu bar 1
            if (count of allLines) >= itemLimit then exit repeat
            set topTitle to name of barItem as text
            if topFilter is "" or topTitle is topFilter then
                set end of allLines to "0" & tab & topTitle & tab & topTitle & tab & "menu_bar" & tab & "true"
                try
                    set childLines to my appendMenuItems(menu 1 of barItem, topTitle, 1, itemLimit - (count of allLines))
                    repeat with childLine in childLines
                        set end of allLines to contents of childLine
                        if (count of allLines) >= itemLimit then exit repeat
                    end repeat
                end try
            end if
        end repeat
    end tell
    return my joinLines(allLines)
end tell
'''
    output = run_osascript(script, timeout=12)
    items = parse_menu_lines(output)
    summary = {
        "app": app_name,
        "top_menu": top_menu,
        "max_items": max_items,
        "truncated": len(items) >= max_items,
        "items": items,
    }
    text = "Computer action: list_menus\n" + json.dumps(summary, ensure_ascii=False, indent=2)
    return ok(text, f"listed {len(items)} menu items")


def select_menu(args, state_dir):
    ensure_macos("select menu")
    app_name = computer_app_name(args)
    menu_path = menu_path_arg(args)
    script = f'''
tell application "System Events"
    {application_process_setup_script(app_name)}
    tell targetProcess
        set frontmost to true
        click {menu_item_reference(menu_path)}
    end tell
end tell
'''
    output = run_osascript(script, timeout=12)
    text = f"Computer action: select_menu\nApp: {app_name}\nMenu path: {' > '.join(menu_path)}"
    if output:
        text += f"\nOutput: {output}"
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"selected {' > '.join(menu_path)}")


def activate_app(args, state_dir):
    ensure_macos("activate app")
    app_name = computer_app_name(args)
    output = run_osascript(f"tell {target_expression(app_name)} to activate", timeout=12)
    text = f"Computer action: activate_app\nActivated: {app_name}"
    if output:
        text += f"\nOutput: {output}"
    if args.get("screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"activated {app_name}")


def quit_app(args, state_dir):
    ensure_macos("quit app")
    app_name = computer_app_name(args)
    output = run_osascript(f"tell {target_expression(app_name)} to quit", timeout=12)
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    text = f"Computer action: quit_app\nQuit requested: {app_name}\nWait after ms: {wait_after_ms}"
    if output:
        text += f"\nOutput: {output}"
    if bool_arg(args, "screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"quit requested for {app_name}")


def open_item_target(args):
    target = str(args.get("target") or "").strip()
    if not target:
        raise RuntimeError("target is required")
    if len(target) > 2048 or "\0" in target or "\n" in target or "\r" in target:
        raise RuntimeError("target is invalid")
    parsed = urllib.parse.urlparse(target)
    if parsed.scheme:
        scheme = parsed.scheme.lower()
        if scheme not in {"http", "https", "file"}:
            raise RuntimeError("target URL scheme must be http, https, or file")
        if scheme == "file":
            if parsed.username or parsed.password:
                raise RuntimeError("file URL credentials are not allowed")
            try:
                port = parsed.port
            except ValueError as exc:
                raise RuntimeError(f"file URL port is invalid: {exc}")
            if port is not None:
                raise RuntimeError("file URL port is not allowed")
            try:
                host = (parsed.hostname or "").lower()
            except ValueError as exc:
                raise RuntimeError(f"file URL host is invalid: {exc}")
            if host and host != "localhost":
                raise RuntimeError("file URL host must be localhost")
            if parsed.params or parsed.query or parsed.fragment:
                raise RuntimeError("file URL cannot include params, query, or fragment")
            path = Path(urllib.parse.unquote(parsed.path or "")).expanduser()
            if not str(path) or any(ord(ch) < 32 or ord(ch) == 127 for ch in str(path)):
                raise RuntimeError("file URL path is invalid")
            if not path.is_absolute():
                raise RuntimeError("file URL path must be absolute")
            return {"input": target, "value": str(path), "kind": "path", "scheme": "file", "exists": path.exists()}
        if any(ch.isspace() for ch in target):
            raise RuntimeError("target URL cannot contain whitespace")
        try:
            hostname = parsed.hostname
        except ValueError as exc:
            raise RuntimeError(f"target URL host is invalid: {exc}")
        if not hostname:
            raise RuntimeError("target URL host is required")
        if parsed.username or parsed.password:
            raise RuntimeError("target URL credentials are not allowed")
        try:
            parsed.port
        except ValueError as exc:
            raise RuntimeError(f"target URL port is invalid: {exc}")
        return {"input": target, "value": target, "kind": "url", "scheme": scheme}
    path = Path(target).expanduser()
    return {"input": target, "value": str(path), "kind": "path", "scheme": "", "exists": path.exists()}


def open_app_args(app_name):
    app_name = str(app_name or "").strip()
    if not app_name:
        return []
    if len(app_name) > 120 or "\n" in app_name or "\r" in app_name:
        raise RuntimeError("app is invalid")
    if app_name.startswith("id:"):
        bundle_id = app_name[3:].strip()
        if not bundle_id:
            raise RuntimeError("app bundle id is required")
        return ["-b", bundle_id]
    if app_name.startswith("app:"):
        app_name = app_name[4:].strip()
    if not app_name:
        raise RuntimeError("app is required")
    return ["-a", app_name]


def open_item(args, state_dir):
    ensure_macos("open item")
    target = open_item_target(args)
    app_name = str(args.get("app") or "").strip()
    reveal = bool_arg(args, "reveal", False)
    if reveal and target.get("kind") != "path":
        raise RuntimeError("reveal requires a file or directory path")
    if reveal and app_name:
        raise RuntimeError("reveal cannot be combined with app")
    command = ["open"]
    if reveal:
        command.append("-R")
    else:
        command.extend(open_app_args(app_name))
    command.append(str(target["value"]))
    completed = subprocess.run(
        command,
        capture_output=True,
        text=True,
        timeout=20,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError((completed.stderr or completed.stdout or "open failed").strip())
    wait_after_ms = int_arg(args, "wait_after_ms", 500, 0, 5000)
    if wait_after_ms:
        time.sleep(wait_after_ms / 1000)
    details = {
        "target": target["input"],
        "resolved_target": target["value"],
        "kind": target["kind"],
        "scheme": target.get("scheme") or "",
        "app": app_name,
        "reveal": reveal,
        "wait_after_ms": wait_after_ms,
    }
    if "exists" in target:
        details["exists"] = bool(target.get("exists"))
    text = "Computer action: open_item\n" + json.dumps(details, ensure_ascii=False, indent=2)
    if bool_arg(args, "screenshot", True):
        shot = take_screenshot({}, state_dir)
        text += f"\nScreenshot: {shot}"
    return ok(text, f"opened {target['input']}")


def handle(payload):
    action = str(payload.get("action") or "")
    args = payload.get("args") or {}
    state_dir = str(payload.get("state_dir") or tempfile.gettempdir())
    if action == "computer_status":
        return computer_status()
    if action == "computer_observe":
        return observe(args, state_dir)
    if action == "computer_list_displays":
        return list_displays(args, state_dir)
    if action == "computer_screenshot":
        return screenshot_result(args, state_dir)
    if action == "computer_screenshot_element":
        return screenshot_element(args, state_dir)
    if action == "computer_screenshot_window":
        return screenshot_window(args, state_dir)
    if action == "computer_read_text":
        return read_text(args, state_dir)
    if action == "computer_find_text":
        return find_text(args, state_dir)
    if action == "computer_move_to_text":
        return move_to_text(args, state_dir)
    if action == "computer_click_text":
        return click_text(args, state_dir)
    if action == "computer_select_text":
        return select_text(args, state_dir)
    if action == "computer_assert_text":
        return assert_text(args, state_dir)
    if action == "computer_wait_for_text":
        return wait_for_text(args, state_dir)
    if action == "computer_wait":
        return wait_for_desktop(args, state_dir)
    if action == "computer_accessibility_snapshot":
        return accessibility_snapshot(args, state_dir)
    if action == "computer_element_at":
        return element_at(args, state_dir)
    if action == "computer_press_at":
        return press_at(args, state_dir)
    if action == "computer_type_at":
        return type_at(args, state_dir)
    if action == "computer_scroll_at":
        return scroll_at(args, state_dir)
    if action == "computer_move_to_element":
        return move_to_element(args, state_dir)
    if action == "computer_click_element":
        return click_element(args, state_dir)
    if action == "computer_press_element":
        return press_element(args, state_dir)
    if action == "computer_focus_element":
        return focus_element(args, state_dir)
    if action == "computer_perform_element_action":
        return perform_element_action(args, state_dir)
    if action == "computer_type_element":
        return type_element(args, state_dir)
    if action == "computer_set_element_value":
        return set_element_value(args, state_dir)
    if action == "computer_assert_element":
        return assert_element(args, state_dir)
    if action == "computer_wait_for_element":
        return wait_for_element(args, state_dir)
    if action == "computer_get_position":
        return get_position(args, state_dir)
    if action == "computer_move_mouse":
        return move_mouse(args, state_dir)
    if action == "computer_mouse_down":
        return mouse_down(args, state_dir)
    if action == "computer_mouse_up":
        return mouse_up(args, state_dir)
    if action == "computer_click":
        return click(args, state_dir)
    if action == "computer_drag":
        return drag(args, state_dir)
    if action == "computer_drag_element":
        return drag_element(args, state_dir)
    if action == "computer_type_text":
        return type_text(args, state_dir)
    if action == "computer_get_clipboard":
        return get_clipboard(args, state_dir)
    if action == "computer_set_clipboard":
        return set_clipboard(args, state_dir)
    if action == "computer_copy_selection":
        return copy_selection(args, state_dir)
    if action == "computer_assert_clipboard":
        return assert_clipboard(args, state_dir)
    if action == "computer_wait_for_clipboard":
        return wait_for_clipboard(args, state_dir)
    if action == "computer_press_key":
        return press_key(args, state_dir)
    if action == "computer_hotkey":
        return hotkey(args, state_dir)
    if action == "computer_key_down":
        return key_down(args, state_dir)
    if action == "computer_key_up":
        return key_up(args, state_dir)
    if action == "computer_scroll":
        return scroll(args, state_dir)
    if action == "computer_scroll_element":
        return scroll_element(args, state_dir)
    if action == "computer_list_apps":
        return list_apps(args, state_dir)
    if action == "computer_assert_app":
        return assert_app(args, state_dir)
    if action == "computer_wait_for_app":
        return wait_for_app(args, state_dir)
    if action == "computer_list_windows":
        return list_windows(args, state_dir)
    if action == "computer_focus_window":
        return focus_window(args, state_dir)
    if action == "computer_set_window_bounds":
        return set_window_bounds(args, state_dir)
    if action == "computer_perform_window_action":
        return perform_window_action(args, state_dir)
    if action == "computer_close_window":
        return close_window(args, state_dir)
    if action == "computer_assert_window":
        return assert_window(args, state_dir)
    if action == "computer_wait_for_window":
        return wait_for_window(args, state_dir)
    if action == "computer_list_menus":
        return list_menus(args, state_dir)
    if action == "computer_select_menu":
        return select_menu(args, state_dir)
    if action == "computer_activate_app":
        return activate_app(args, state_dir)
    if action == "computer_quit_app":
        return quit_app(args, state_dir)
    if action == "computer_open_item":
        return open_item(args, state_dir)
    raise RuntimeError(f"unknown computer action: {action}")


try:
    payload = json.loads(sys.stdin.read() or "{}")
    result = handle(payload)
except Exception as exc:
    result = err(f"Computer tool failed: {exc}")
print(json.dumps(result, ensure_ascii=False), flush=True)
