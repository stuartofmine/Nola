// SPDX-License-Identifier: Apache-2.0
//! Executable identity is compiled in; renaming the Host cannot grant CLI commands.
use std::{process::{Command, Stdio}, thread, time::{Duration, Instant}};

fn bounded_output(mut command:Command)->std::process::Output {
    let mut child=command.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null()).spawn().unwrap();
    let deadline=Instant::now()+Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some(){return child.wait_with_output().unwrap();}
        if Instant::now()>=deadline {
            let _=child.kill();let _=child.wait();panic!("entry validation did not terminate before dispatch");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn renamed_host_keeps_its_command_boundary_despite_cli_environment() {
    let directory=tempfile::tempdir().unwrap();
    let renamed=directory.path().join(if cfg!(windows){"nola.exe"}else{"nola"});
    std::fs::copy(env!("CARGO_BIN_EXE_nola-agent-host"),&renamed).unwrap();
    for command in ["run","init","wiki","web"] {
        let mut process=Command::new(&renamed);
        process.current_dir(directory.path()).arg(command).args(["--endpoint","http://127.0.0.1:1/v1","--model","fixture"])
            .env("NOLA_HOME",directory.path()).env("NOLA_DB_PATH",directory.path().join("unused.db"))
            .env("NOLA_CLIENT_SURFACE","cli").env("NOLA_PERMISSION_PROFILE","cautious")
            .env_remove("NOLA_DESKTOP_HOST");
        let output=bounded_output(process);
        assert_eq!(output.status.code(),Some(1),"{command}: {}",String::from_utf8_lossy(&output.stderr));
        let error=String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("requires native stdin for runs; unsupported product command"),"{command}: {error}");
    }
    assert!(!directory.path().join("unused.db").exists(),"rejected commands must not initialize user storage");
}

#[test]
fn renamed_host_keeps_the_explicit_same_build_service_entry() {
    let directory=tempfile::tempdir().unwrap();
    let renamed=directory.path().join(if cfg!(windows){"worker.exe"}else{"worker"});
    std::fs::copy(env!("CARGO_BIN_EXE_nola-agent-host"),&renamed).unwrap();
    let mut process=Command::new(&renamed);
    process.current_dir(directory.path()).arg("--nola-application-service").env("NOLA_HOME",directory.path());
    let output=bounded_output(process);
    assert_eq!(output.status.code(),Some(1));
    let error=String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("nola application service:"),"{error}");
    assert!(!error.contains("supports desktop run requests"));
}
