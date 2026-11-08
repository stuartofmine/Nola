// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Wrapper script that parses arguments and delegates to build.sh
// - Usage: ./wrapper [build_options] -- [nola_args]
//   Example: ./wrapper --debug --run -- nola_arg1 nola_arg2

use std::env;
use std::path::PathBuf;
use std::process::{self, Command};

fn print_help() {
    let help_text = "\
Usage: ./wrapper [options] -- [args]

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
  ./wrapper --debug --run -- nola_arg1 nola_arg2
  ./wrapper --target x86_64-unknown-linux-gnu --features json -- 
  ./wrapper --clean -- nola_help
";
    println!("{help_text}");
}

fn find_build_script() -> Result<PathBuf, String> {
    if let Some(explicit) = env::var_os("NOLA_CLI_BUILD_SCRIPT") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "NOLA_CLI_BUILD_SCRIPT does not point to a file: {}",
            path.display()
        ));
    }

    let mut candidates = Vec::new();
    if let Ok(executable) = env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.join("build.sh"));
            if let Some(grandparent) = parent.parent() {
                candidates.push(grandparent.join("scripts").join("build.sh"));
            }
        }
    }
    if let Ok(current) = env::current_dir() {
        candidates.push(current.join("scripts").join("build.sh"));
        candidates.push(current.join("build.sh"));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "build.sh was not found; run from the CLI repository or set NOLA_CLI_BUILD_SCRIPT"
                .to_string()
        })
}

fn main() {
    let build_script = find_build_script().unwrap_or_else(|error| {
        eprintln!("Error: {error}");
        process::exit(1);
    });

    let mut build_args: Vec<String> = Vec::new();
    let mut run_args: Vec<String> = Vec::new();

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--debug" => {
                build_args.push(arg.clone());
                i += 1;
            }
            "--release" => {
                build_args.push(arg.clone());
                i += 1;
            }
            "--target" | "--targets" | "--preset" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: Missing value for {arg}");
                    process::exit(1);
                }
                build_args.push(arg.clone());
                build_args.push(args[i + 1].clone());
                i += 2;
            }
            "--install-targets" => {
                build_args.push(arg.clone());
                i += 1;
            }
            "--features" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: Missing value for --features");
                    process::exit(1);
                }
                build_args.push(arg.clone());
                build_args.push(args[i + 1].clone());
                i += 2;
            }
            "--clean" => {
                build_args.push(arg.clone());
                i += 1;
            }
            "--run" => {
                build_args.push(arg.clone());
                i += 1;
            }
            "--" => {
                // Everything after -- is passed to nola
                run_args = args[i + 1..].to_vec();
                break;
            }
            "-h" | "--help" => {
                print_help();
                process::exit(0);
            }
            _ => {
                eprintln!("Unknown option: {arg}");
                print_help();
                process::exit(1);
            }
        }
    }

    // Build command to pass to build.sh
    let mut cmd = Command::new("bash");
    cmd.arg(&build_script);

    if !build_args.is_empty() {
        cmd.args(&build_args);
        if !run_args.is_empty() {
            cmd.arg("--");
            cmd.args(&run_args);
        }
        println!("Running build with args: {}", build_args.join(" "));
    } else if !run_args.is_empty() {
        // No build args, just pass runtime args directly to build.sh
        cmd.args(["--run", "--"]);
        cmd.args(&run_args);
        println!("Running build with runtime args: {}", run_args.join(" "));
    }

    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("Error: Failed to execute build.sh: {e}");
        process::exit(1);
    });

    if !status.success() {
        process::exit(status.code().unwrap_or(1));
    }
}
