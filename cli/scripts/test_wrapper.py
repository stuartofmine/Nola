#!/usr/bin/env python3
"""Tests for wrapper.py — argument parsing and delegation logic."""

import os
import subprocess
import sys
from unittest.mock import patch, MagicMock, call
import pytest

# Import the module under test
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import wrapper


# ── Helpers ──────────────────────────────────────────────────────────────

def _run_main(argv: list[str]) -> tuple[int, str, str]:
    """Run wrapper.main() with given argv, return (exit_code, stdout, stderr)."""
    with patch.object(sys, "argv", ["wrapper.py"] + argv), \
         patch("sys.stdout.write") as mock_out, \
         patch("sys.stderr.write") as mock_err:
        try:
            wrapper.main()
        except SystemExit as e:
            return (e.code if isinstance(e.code, int) else 1,
                    _captured(mock_out), _captured(mock_err))
    return (0, _captured(mock_out), _captured(mock_err))


def _captured(mock) -> str:
    return "".join(call_args[0][0] for call_args in mock.call_args_list)


# ── Tests: Help ──────────────────────────────────────────────────────────

def test_help_short():
    code, out, err = _run_main(["-h"])
    assert code == 0
    assert "Usage:" in out


def test_help_long():
    code, out, err = _run_main(["--help"])
    assert code == 0
    assert "Usage:" in out


# ── Tests: Unknown option ────────────────────────────────────────────────

def test_unknown_option():
    code, out, err = _run_main(["--bogus"])
    assert code == 1
    assert "Unknown option: --bogus" in err


# ── Tests: Missing option values ─────────────────────────────────────────

def test_target_missing_value():
    code, out, err = _run_main(["--target"])
    assert code == 1
    assert "Missing value for --target" in err


def test_features_missing_value():
    code, out, err = _run_main(["--features"])
    assert code == 1
    assert "Missing value for --features" in err


@pytest.mark.parametrize("option", ["--targets", "--preset"])
def test_multi_target_option_missing_value(option):
    code, out, err = _run_main([option])
    assert code == 1
    assert f"Missing value for {option}" in err


# ── Tests: No arguments ──────────────────────────────────────────────────

@patch("wrapper.subprocess.run")
@patch.object(wrapper.os.path, "isfile", return_value=True)
def test_no_args_runs_default_build(mock_isfile, mock_run):
    mock_run.return_value.returncode = 0
    code, out, err = _run_main([])
    assert code == 0
    cmd = mock_run.call_args[0][0]
    assert cmd[0] == "bash"
    assert cmd[1].endswith("build.sh")
    assert len(cmd) == 2


# ── Tests: build.sh not found ────────────────────────────────────────────

def test_build_script_missing():
    with patch.object(wrapper.os.path, "isfile", return_value=False):
        with patch.object(sys, "argv", ["wrapper.py", "--debug"]):
            with pytest.raises(SystemExit) as exc:
                wrapper.main()
            assert exc.value.code == 1


# ── Tests: Argument parsing (build_args / run_args) ──────────────────────

@pytest.mark.parametrize("argv,expected_build,expected_run", [
    (["--debug"], ["--debug"], []),
    (["--release"], ["--release"], []),
    (["--clean"], ["--clean"], []),
    (["--run"], ["--run"], []),
    (["--debug", "--run"], ["--debug", "--run"], []),
    (["--target", "aarch64-apple-darwin"], ["--target", "aarch64-apple-darwin"], []),
    (["--targets", "x86_64-unknown-linux-gnu,aarch64-unknown-linux-gnu"], ["--targets", "x86_64-unknown-linux-gnu,aarch64-unknown-linux-gnu"], []),
    (["--preset", "linux"], ["--preset", "linux"], []),
    (["--install-targets"], ["--install-targets"], []),
    (["--features", "json,extra"], ["--features", "json,extra"], []),
    (["--debug", "--", "arg1", "arg2"], ["--debug"], ["arg1", "arg2"]),
    (["--", "arg1"], [], ["arg1"]),
    (["--debug", "--run", "--", "x", "y"], ["--debug", "--run"], ["x", "y"]),
])
def test_arg_parsing(argv, expected_build, expected_run):
    """Verify build_args and run_args are populated correctly."""
    with patch.object(sys, "argv", ["wrapper.py"] + argv):
        build_args: list[str] = []
        run_args: list[str] = []

        args = sys.argv[1:]
        i = 0
        while i < len(args):
            arg = args[i]
            if arg == "--debug":
                build_args.append("--debug"); i += 1
            elif arg == "--release":
                build_args.append("--release"); i += 1
            elif arg in ("--target", "--targets", "--preset"):
                build_args.extend([arg, args[i+1]]); i += 2
            elif arg == "--install-targets":
                build_args.append(arg); i += 1
            elif arg == "--features":
                build_args.extend(["--features", args[i+1]]); i += 2
            elif arg == "--clean":
                build_args.append("--clean"); i += 1
            elif arg == "--run":
                build_args.append("--run"); i += 1
            elif arg == "--":
                run_args = args[i+1:]; break
            elif arg in ("-h", "--help"):
                break
            else:
                break

        assert build_args == expected_build
        assert run_args == expected_run


# ── Tests: subprocess call construction ──────────────────────────────────

@patch("wrapper.subprocess.run")
@patch.object(wrapper.os.path, "isfile", return_value=True)
def test_subprocess_with_build_and_run_args(mock_isfile, mock_run):
    """build_args + run_args → cmd includes -- separator."""
    mock_run.return_value.returncode = 0
    with patch.object(sys, "argv", ["wrapper.py", "--debug", "--", "nola_arg"]):
        wrapper.main()
    mock_run.assert_called_once()
    cmd = mock_run.call_args[0][0]
    assert cmd[0] == "bash"
    assert cmd[1].endswith("build.sh")
    assert "--debug" in cmd
    assert "--" in cmd
    assert "nola_arg" in cmd


@patch("wrapper.subprocess.run")
@patch.object(wrapper.os.path, "isfile", return_value=True)
def test_subprocess_build_only(mock_isfile, mock_run):
    """Only build_args, no -- separator."""
    mock_run.return_value.returncode = 0
    with patch.object(sys, "argv", ["wrapper.py", "--release", "--clean"]):
        wrapper.main()
    cmd = mock_run.call_args[0][0]
    assert "--release" in cmd
    assert "--clean" in cmd
    assert "--" not in cmd


@patch("wrapper.subprocess.run")
@patch.object(wrapper.os.path, "isfile", return_value=True)
def test_subprocess_runtime_only(mock_isfile, mock_run):
    """No build_args, only run_args → injects --run."""
    mock_run.return_value.returncode = 0
    with patch.object(sys, "argv", ["wrapper.py", "--", "nola_help"]):
        wrapper.main()
    cmd = mock_run.call_args[0][0]
    assert "--run" in cmd
    assert "--" in cmd
    assert "nola_help" in cmd


@patch("wrapper.subprocess.run")
@patch.object(wrapper.os.path, "isfile", return_value=True)
def test_subprocess_failure_exit_code_is_propagated(mock_isfile, mock_run):
    mock_run.return_value.returncode = 17
    with patch.object(sys, "argv", ["wrapper.py", "--debug"]):
        with pytest.raises(SystemExit) as exc:
            wrapper.main()
    assert exc.value.code == 17


@patch("wrapper.subprocess.run", side_effect=OSError("shell missing"))
@patch.object(wrapper.os.path, "isfile", return_value=True)
def test_subprocess_start_failure_is_reported(mock_isfile, mock_run):
    with patch.object(sys, "argv", ["wrapper.py", "--debug"]):
        with pytest.raises(SystemExit) as exc:
            wrapper.main()
    assert exc.value.code == 1


# ── Tests: print_help standalone ─────────────────────────────────────────

def test_print_help_output():
    with patch("sys.stdout.write") as mock_out:
        wrapper.print_help()
    output = _captured(mock_out)
    assert "Usage:" in output
    assert "--debug" in output
    assert "--target" in output
    assert "--preset" in output
