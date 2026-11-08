"""Unit boundaries only; the separate Wrangler runner proves real local HTTP."""
import os
from pathlib import Path
from unittest import mock

import pytest

import test_messaging_webhooks_e2e as fixture


def test_fixture_uses_private_config_empty_env_and_sanitized_child_environment(tmp_path):
    original_home = tmp_path / "original-home"
    original_home.mkdir()
    environment = {"HOME": str(original_home), "PATH": os.defpath, "LANG": "en_US.UTF-8",
                   "CLOUDFLARE_API_TOKEN": "must-not-propagate", "NOLA_BACKEND_ENV": "must-not-propagate",
                   "HTTP_PROXY": "http://must-not-propagate.invalid", "NODE_OPTIONS": "must-not-propagate"}
    root = tmp_path / "fixture"
    root.mkdir()
    with mock.patch.dict(os.environ, environment, clear=True):
        child = fixture.prepare_fixture(root)
    assert child["HOME"] == str(original_home)
    for secret in ("CLOUDFLARE_API_TOKEN", "NOLA_BACKEND_ENV", "HTTP_PROXY", "NODE_OPTIONS"):
        assert secret not in child
    assert child["WRANGLER_SEND_METRICS"] == child["WRANGLER_SEND_ERROR_REPORTS"] == "false"
    assert child["CLOUDFLARE_LOAD_DEV_VARS_FROM_DOT_ENV"] == child["CLOUDFLARE_INCLUDE_PROCESS_ENV"] == "false"
    for key in ("XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "TMPDIR"):
        assert Path(child[key]).is_relative_to(root)
    assert (root / "empty.env").read_bytes() == b""
    config = (root / "wrangler.toml").read_text()
    assert str(fixture.WORKER / "src/index.ts") in config
    assert str(fixture.WORKER / "migrations") in config
    assert fixture.FIXTURE_DATABASE in config
    assert "[[routes]]" not in config and "nola.top" not in config and "[vars]" not in config


def test_legacy_wrangler_credentials_block_execution_without_reading_contents(tmp_path):
    legacy = tmp_path / ".wrangler"
    legacy.mkdir()
    with mock.patch.dict(os.environ, {"HOME": str(tmp_path)}, clear=True), \
            mock.patch.object(Path, "read_text", side_effect=AssertionError("credentials must not be read")):
        with pytest.raises(RuntimeError, match="legacy"):
            fixture.isolated_environment(tmp_path / "unused")


def test_every_wrangler_command_pins_local_database_config_and_env(tmp_path):
    for arguments in [("dev", "--ip", "127.0.0.1"),
                      ("d1", "migrations", "apply", fixture.FIXTURE_DATABASE),
                      ("d1", "execute", fixture.FIXTURE_DATABASE, "--command", "SELECT 1")]:
        command = fixture.wrangler_command(tmp_path, *arguments)
        assert command[command.index("--config") + 1] == str(tmp_path / "wrangler.toml")
        assert command[-1] == f"--env-file={tmp_path / 'empty.env'}"
        assert command[command.index("--persist-to") + 1] == str(tmp_path / "state")
        assert "--local" in command
    for arguments in [("deploy",), ("dev", "--remote"), ("dev", "--env-file=private.env"),
                      ("d1", "execute", "production-database")]:
        with pytest.raises(ValueError):
            fixture.wrangler_command(tmp_path, *arguments)


@pytest.mark.parametrize("arguments", [
    ("dev", "--ip", "127.0.0.1", "--var", "SYNTHETIC_TOKEN:test-token"),
    ("d1", "migrations", "apply", fixture.FIXTURE_DATABASE),
    ("d1", "execute", fixture.FIXTURE_DATABASE, "--command", "SELECT 1", "--json"),
])
def test_array_env_file_is_explicitly_bound_after_all_command_arguments(tmp_path, arguments):
    command = fixture.wrangler_command(tmp_path, *arguments)
    assert command == [
        str(fixture.WRANGLER), "--config", str(tmp_path / "wrangler.toml"), *arguments,
        "--local", "--persist-to", str(tmp_path / "state"),
        f"--env-file={tmp_path / 'empty.env'}",
    ]
    assert sum(argument.startswith("--env-file") for argument in command) == 1


def test_http_transport_rejects_nonloopback_and_disables_proxy_and_redirect():
    for url in ("https://example.com", "http://localhost:8080", "http://127.0.0.1:8080@elsewhere.invalid",
                "http://user@127.0.0.1:8080", "http://127.0.0.1:8080/#fragment"):
        with pytest.raises(ValueError):
            fixture.request(url)
    opener = mock.MagicMock()
    response = opener.open.return_value.__enter__.return_value
    response.status = 200
    response.read.return_value = b"owned"
    with mock.patch.object(fixture.urllib.request, "build_opener", return_value=opener) as build:
        assert fixture.request("http://127.0.0.1:8080/healthz") == (200, "owned")
    proxy, redirect = build.call_args.args
    assert isinstance(proxy, fixture.urllib.request.ProxyHandler) and proxy.proxies == {}
    assert isinstance(redirect, fixture.NoRedirect)
    assert redirect.redirect_request(None, None, 302, "moved", {}, "https://elsewhere.invalid") is None
