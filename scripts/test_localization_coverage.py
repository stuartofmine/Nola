import importlib.util
import copy
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("localization_coverage.py")
SPEC = importlib.util.spec_from_file_location("localization_coverage", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
localization = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(localization)


class LocalizationCoverageTests(unittest.TestCase):
    def test_swift_concatenated_key_is_not_a_literal_prefix(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "View.swift").write_text(
                'NSLocalizedString("reasoning.effort." + value, comment: "")\n'
                'String(localized: "dynamic."\n + suffix)\n'
                'LocalizedStringKey("interpolated.\\(value)")\n'
                'Text("screen.title")\nNSLocalizedString("missing.key", comment: "")'
            )
            self.assertEqual(localization.swift_keys(root), {"screen.title", "missing.key"})

    def web_snapshot(self):
        catalogs = [{locale: {"screen.title": f"Translated {locale} {{name}}"}
                     for locale in localization.WEB_LOCALES}]
        return {"locales": list(localization.WEB_LOCALES), "catalogs": catalogs,
                "sourceKeys": [{"key": "screen.title", "file": "App.svelte"}], "sourceFiles": 1}

    def test_web_audits_current_runtime_registry_and_svelte_source(self):
        self.assertEqual(localization.audit_web_snapshot(self.web_snapshot(), False), [])

    def test_web_rejects_incomplete_registry_source_and_interpolation(self):
        for fault in ("empty", "locale", "missing", "extra", "duplicate", "source_duplicate", "placeholder", "source", "unregistered"):
            with self.subTest(fault=fault):
                snapshot = self.web_snapshot()
                if fault == "empty": snapshot["catalogs"] = []
                elif fault == "locale": snapshot["locales"].pop()
                elif fault == "missing": del snapshot["catalogs"][0]["fr"]["screen.title"]
                elif fault == "extra": snapshot["catalogs"][0]["de"]["extra.key"] = "Extra"
                elif fault == "duplicate": snapshot["catalogs"].append(copy.deepcopy(snapshot["catalogs"][0]))
                elif fault == "source_duplicate": snapshot["duplicateKeys"] = [{"file": "lib/locales/example.ts", "key": "title"}]
                elif fault == "placeholder": snapshot["catalogs"][0]["ja"]["screen.title"] = "Translated {wrong}"
                elif fault == "source": snapshot["sourceKeys"] = []
                elif fault == "unregistered": snapshot["sourceKeys"][0]["key"] = "unknown.key"
                self.assertTrue(localization.audit_web_snapshot(snapshot, False))

    def test_web_quality_still_rejects_copied_english_sentence(self):
        snapshot = self.web_snapshot()
        sentence = "This setting controls background processing."
        snapshot["catalogs"][0]["en"]["screen.title"] = sentence
        snapshot["catalogs"][0]["fr"]["screen.title"] = sentence
        self.assertIn("web: screen.title: untranslated fr", localization.audit_web_snapshot(snapshot, True))

    def test_apple_routing_supports_strings_without_masking_wrong_table(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "en.lproj").mkdir()
            (root / "en.lproj/Localizable.strings").write_text('"hello" = "Hello";\n')
            (root / "View.swift").write_text('String(localized: "hello")\nString(localized: "hello", table: "Chat")')
            failures = localization.audit_apple_catalog_routing("ios", root, root)
            self.assertEqual(len(failures), 1)
            self.assertIn("missing catalog table Chat", failures[0])

    def test_android_placeholders_ignore_positions_but_preserve_types(self) -> None:
        self.assertEqual(localization.android_placeholders("%2$d · %1$s · %%"), ["d", "s"])

    def test_android_audit_rejects_locale_placeholder_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            resources = root / "android/app/src/main/res"
            locale_dirs = ("de", "es", "fr", "ja", "ru", "zh")
            default = resources / "values/strings.xml"
            default.parent.mkdir(parents=True)
            default.write_text(
                '<resources><string name="failure">Failed: %1$s</string></resources>',
                encoding="utf-8",
            )
            for locale in locale_dirs:
                path = resources / f"values-{locale}/strings.xml"
                path.parent.mkdir(parents=True)
                value = "Fehler" if locale == "zh" else "Fehler: %1$s"
                path.write_text(
                    f'<resources><string name="failure">{value}</string></resources>',
                    encoding="utf-8",
                )

            original_root = localization.ROOT
            localization.ROOT = root
            try:
                failures = localization.audit_android()
            finally:
                localization.ROOT = original_root

            self.assertEqual(failures, ["android: failure: placeholder mismatch in zh-Hans"])

    def test_executable_command_is_not_treated_as_localizable_copy(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "View.swift").write_text(
                'Text("nola web --host 0.0.0.0 --port 5001")\nText("chat.real_key")',
                encoding="utf-8",
            )
            self.assertEqual(localization.swift_keys(root), {"chat.real_key"})

    def test_stable_product_name_is_allowed_in_japanese(self) -> None:
        self.assertFalse(localization.obviously_untranslated("WeCom", "WeCom", "ja"))

    def test_android_quality_rejects_copied_english_sentence(self) -> None:
        self.assertTrue(
            localization.obviously_untranslated(
                "This setting controls background processing.",
                "This setting controls background processing.",
                "zh-Hans",
            )
        )


if __name__ == "__main__":
    unittest.main()
