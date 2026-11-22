#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Mutation tests proving scripts/check_design_tokens.py catches cross-platform surface and token regressions."""

from __future__ import annotations

import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import check_design_tokens as cdt


REPO_ROOT = SCRIPTS_DIR.parent
RELEVANT_PATHS = (
    "shared/design-tokens/tokens.json",
    "docs/design/prototype",
    "apps/web/frontend/src",
    "apps/android/app/src/main/java/com/nola/ui",
    "apps/android/app/src/main/res",
    "apps/windows/src",
    "apps/windows/scripts/export-shell-strings.mjs",
    "apps/ios",
    "apps/macos",
)


def copy_checked_tree(source_root: Path, dest_root: Path) -> None:
    for rel in RELEVANT_PATHS:
        src = source_root / rel
        dst = dest_root / rel
        if src.is_file():
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dst)
        elif src.is_dir():
            if rel == "apps/android/app/src/main/res":
                for values_dir in src.glob("values*"):
                    if values_dir.is_dir():
                        shutil.copytree(values_dir, dst / values_dir.name)
            elif rel in {"apps/ios", "apps/macos"}:
                assets = src / "Assets.xcassets"
                if assets.is_dir():
                    shutil.copytree(assets, dst / "Assets.xcassets")
                for swift_file in src.rglob("*.swift"):
                    target = dst / swift_file.relative_to(src)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(swift_file, target)
            elif rel == "apps/web/frontend/src":
                for item in src.rglob("*"):
                    if not item.is_file():
                        continue
                    rel_item = item.relative_to(src)
                    if "vendor" in rel_item.parts or ".test." in item.name:
                        continue
                    if item.suffix in {".ts", ".svelte", ".css"}:
                        target = dst / rel_item
                        target.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copy2(item, target)
            else:
                shutil.copytree(src, dst)


class TestCheckDesignTokens(unittest.TestCase):
    _template_dir: tempfile.TemporaryDirectory[str]
    _template_root: Path

    @classmethod
    def setUpClass(cls) -> None:
        cls._template_dir = tempfile.TemporaryDirectory(prefix="nola-token-template-")
        cls._template_root = Path(cls._template_dir.name)
        copy_checked_tree(REPO_ROOT, cls._template_root)

    @classmethod
    def tearDownClass(cls) -> None:
        cls._template_dir.cleanup()

    def make_workspace(self) -> Path:
        temp_dir = tempfile.TemporaryDirectory(prefix="nola-token-case-")
        self.addCleanup(temp_dir.cleanup)
        workspace = Path(temp_dir.name) / "repo"
        shutil.copytree(self._template_root, workspace)
        return workspace

    def test_baseline_passes(self) -> None:
        workspace = self.make_workspace()
        errors, schema_version, contrast_checks = cdt.run_checks(workspace)
        self.assertEqual(errors, [])
        self.assertEqual(schema_version, 1)
        self.assertGreater(contrast_checks, 0)

    def test_xcassets_pure_black_fails(self) -> None:
        workspace = self.make_workspace()
        colorset = workspace / "apps/macos/Assets.xcassets/NolaCanvas.colorset/Contents.json"
        payload = json.loads(colorset.read_text(encoding="utf-8"))
        for entry in payload["colors"]:
            if entry.get("appearances"):
                entry["color"]["components"].update({"red": "0x00", "green": "0x00", "blue": "0x00"})
        colorset.write_text(json.dumps(payload), encoding="utf-8")

        errors, _, _ = cdt.run_checks(workspace)
        self.assertTrue(
            any("NolaCanvas.colorset/Contents.json" in err and "#000000" in err for err in errors),
            f"Expected xcassets #000000 failure, got: {errors}",
        )

    def test_macos_forbidden_appkit_background_fails(self) -> None:
        workspace = self.make_workspace()
        target = workspace / "apps/macos/UI/MacPageChrome.swift"
        target.write_text(
            target.read_text(encoding="utf-8") + "\nlet leaked = NSColor.windowBackgroundColor\n",
            encoding="utf-8",
        )

        errors, _, _ = cdt.run_checks(workspace)
        self.assertTrue(
            any("MacPageChrome.swift" in err and "windowBackgroundColor" in err for err in errors),
            f"Expected macOS windowBackgroundColor failure, got: {errors}",
        )

    def test_ios_forbidden_system_grouped_background_fails(self) -> None:
        workspace = self.make_workspace()
        target = workspace / "apps/ios/UI/DesignSystem.swift"
        target.write_text(
            target.read_text(encoding="utf-8") + "\nlet leaked = Color(.systemGroupedBackground)\n",
            encoding="utf-8",
        )

        errors, _, _ = cdt.run_checks(workspace)
        self.assertTrue(
            any("DesignSystem.swift" in err and "systemGroupedBackground" in err for err in errors),
            f"Expected iOS systemGroupedBackground failure, got: {errors}",
        )

    def test_android_oled_black_fails_but_translucent_alpha_allowed(self) -> None:
        workspace = self.make_workspace()
        kt_file = workspace / "apps/android/app/src/main/java/com/nola/ui/components/DesignSystem.kt"
        original = kt_file.read_text(encoding="utf-8")
        kt_file.write_text(original + "\nval scrim = Color(0x20000000)\n", encoding="utf-8")
        errors, _, _ = cdt.run_checks(workspace)
        self.assertEqual(errors, [], f"Translucent 0x20000000 must not fail: {errors}")

        kt_file.write_text(original + "\nval oledLeak = Color(0xFF000000)\n", encoding="utf-8")
        errors, _, _ = cdt.run_checks(workspace)
        self.assertTrue(
            any("DesignSystem.kt" in err and "0xFF000000" in err for err in errors),
            f"Expected Android 0xFF000000 failure, got: {errors}",
        )

    def test_windows_missing_role_fails(self) -> None:
        workspace = self.make_workspace()
        tokens_ts = workspace / "apps/windows/src/themeTokens.ts"
        updated = tokens_ts.read_text(encoding="utf-8").replace('    surfaceElevated: "#ffffff",\n', "")
        tokens_ts.write_text(updated, encoding="utf-8")

        errors, _, _ = cdt.run_checks(workspace)
        self.assertTrue(
            any("themeTokens.ts" in err and "surfaceElevated" in err for err in errors),
            f"Expected themeTokens.ts surfaceElevated failure, got: {errors}",
        )

    def test_web_missing_a2ui_surface_mapping_fails(self) -> None:
        workspace = self.make_workspace()
        theme_css = workspace / "apps/web/frontend/src/theme.css"
        updated = theme_css.read_text(encoding="utf-8").replace("--a2ui-color-surface: var(--surface);", "")
        theme_css.write_text(updated, encoding="utf-8")

        errors, _, _ = cdt.run_checks(workspace)
        self.assertTrue(
            any("theme.css" in err and "a2ui-color-surface" in err for err in errors),
            f"Expected theme.css --a2ui-color-surface failure, got: {errors}",
        )

    def test_expert_bubble_low_contrast_fails(self) -> None:
        workspace = self.make_workspace()
        bubble_ts = workspace / "apps/web/frontend/src/lib/expertBubbleColors.ts"
        updated = bubble_ts.read_text(encoding="utf-8").replace("#F8F1F1", "#181817")
        bubble_ts.write_text(updated, encoding="utf-8")

        errors, _, _ = cdt.run_checks(workspace)
        self.assertTrue(
            any("expertBubbleColors.ts" in err and "contrast" in err for err in errors),
            f"Expected expertBubbleColors.ts contrast failure, got: {errors}",
        )

    def test_web_cold_surface_hex_fails(self) -> None:
        workspace = self.make_workspace()
        diagram_ts = workspace / "apps/web/frontend/src/lib/diagramDocument.ts"
        diagram_ts.write_text(
            diagram_ts.read_text(encoding="utf-8").replace(
                json.loads((workspace / "shared/design-tokens/tokens.json").read_text())["color"]["dark"]["canvas"].lower(),
                "#14161b",
            ),
            encoding="utf-8",
        )

        errors, _, _ = cdt.run_checks(workspace)
        self.assertTrue(
            any("diagramDocument.ts" in err and "#14161b" in err for err in errors),
            f"Expected diagramDocument.ts cold surface failure, got: {errors}",
        )


if __name__ == "__main__":
    unittest.main()
