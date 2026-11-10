#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Wrapper script that parses arguments and delegates to build.sh
# - Usage: ./wrapper.py [build_options] -- [nola_args]
#   Example: ./wrapper.py --debug --run -- nola_arg1 nola_arg2

import os
import subprocess
import sys


def print_help() -> None:
    help_text = """\
Usage: ./wrapper.py [options] -- [args]

Wrapper script for build.sh with argument forwarding.

Build Options:
  --debug                  Build in debug mode
  --release                Build in release mode (default)
  --target <triple>        Build for a specific target triple
  --targets <list>         Build comma-separated target triples
  --preset <name>          Build a target preset: macos, linux, windows, all
  --install-targets       Allow installing missing Rust targets
  --features <feature..>   Pass cargo --features argument
  --clean                  Run cargo clean before building
  --run                    Build then start Nola immediately

Argument Forwarding:
  --                       Separate build flags from runtime args
  [args...]                Arguments passed to nola after build

Examples:
  ./wrapper.py --debug --run -- nola_arg1 nola_arg2
  ./wrapper.py --target x86_64-unknown-linux-gnu --features json -- 
  ./wrapper.py --clean -- nola_help
"""
    print(help_text)


def main() -> None:
    script_dir = os.path.dirname(os.path.abspath(__file__))
    build_script = os.path.join(script_dir, "build.sh")

    build_args: list[str] = []
    run_args: list[str] = []

    args = sys.argv[1:]
    i = 0
    while i < len(args):
        arg = args[i]
        if arg == "--debug":
            build_args.append("--debug")
            i += 1
        elif arg == "--release":
            build_args.append("--release")
            i += 1
        elif arg in ("--target", "--targets", "--preset"):
            if i + 1 >= len(args):
                print(f"Error: Missing value for {arg}", file=sys.stderr)
                sys.exit(1)
            build_args.extend([arg, args[i + 1]])
            i += 2
        elif arg == "--install-targets":
            build_args.append("--install-targets")
            i += 1
        elif arg == "--features":
            if i + 1 >= len(args):
                print("Error: Missing value for --features", file=sys.stderr)
                sys.exit(1)
            build_args.extend(["--features", args[i + 1]])
            i += 2
        elif arg == "--clean":
            build_args.append("--clean")
            i += 1
        elif arg == "--run":
            build_args.append("--run")
            i += 1
        elif arg == "--":
            # Everything after -- is passed to nola
            run_args = args[i + 1 :]
            break
        elif arg in ("-h", "--help"):
            print_help()
            sys.exit(0)
        else:
            print(f"Unknown option: {arg}", file=sys.stderr)
            print_help()
            sys.exit(1)

    # Validate that build.sh exists
    if not os.path.isfile(build_script):
        print(f"Error: build.sh not found at {build_script}", file=sys.stderr)
        sys.exit(1)

    # Build command to pass to build.sh
    cmd: list[str] = ["bash", build_script]

    if build_args:
        cmd.extend(build_args)
        if run_args:
            cmd.extend(["--"] + run_args)
        print(f"Running build with args: {' '.join(build_args)}")
    elif run_args:
        # No build args, just pass runtime args directly to build.sh
        cmd.extend(["--run", "--"] + run_args)
        print(f"Running build with runtime args: {' '.join(run_args)}")

    try:
        completed = subprocess.run(cmd)
    except OSError as error:
        print(f"Error: Failed to execute build.sh: {error}", file=sys.stderr)
        raise SystemExit(1) from error
    if completed.returncode != 0:
        raise SystemExit(completed.returncode)


if __name__ == "__main__":
    main()
