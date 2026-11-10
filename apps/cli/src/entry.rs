// SPDX-License-Identifier: Apache-2.0
//! Compile-time-selected executable identity, separate from caller provenance.
//! argv[0], filesystem names and environment variables cannot select this enum.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryIdentity {
    Cli,
    AgentHost,
}

#[cfg(any(feature = "terminal-ui", test))]
pub fn run_cli() {
    let arguments=std::env::args().skip(1).collect::<Vec<_>>();
    // Updating must work even when the model or conversation configuration is invalid.
    if let Some(result) = crate::software_update::run_if_requested(&arguments) {
        if let Err(error) = result { eprintln!("{error}"); std::process::exit(1); }
        return;
    }
    if let Some(result)=crate::tailcat_web_client::run_if_requested(&arguments) {
        if let Err(error)=result {eprintln!("{error}");std::process::exit(1);}
        return;
    }
    if let Some(result)=crate::tailcat_transport::run_if_requested(&arguments) {
        if let Err(error)=result {eprintln!("{error}");std::process::exit(1);}
        return;
    }
    crate::CliApplication::run(EntryIdentity::Cli);
}

pub fn run_agent_host() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if crate::native_host_launch::is_native_run(&arguments) {
        crate::native_host_launch::configure_surface();
        if let Err(error) = crate::native_host_launch::run(&arguments) {
            if !error.is_empty() { eprintln!("{error}"); }
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = crate::native_host_commands::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_host_identity_limits_product_commands_and_keeps_native_modes() {
        let _environment=crate::store::test_env_lock();
        let mut config=crate::config::Config::for_test().unwrap();
        config.show_help=false;
        config.command=crate::config::CommandMode::Run;
        config.host_request_stdin=false;
        assert!(crate::validate_agent_host_invocation(EntryIdentity::AgentHost,&config).is_err());
        assert!(crate::validate_agent_host_invocation(EntryIdentity::Cli,&config).is_ok());
        config.host_request_stdin=true;
        assert!(crate::validate_agent_host_invocation(EntryIdentity::AgentHost,&config).is_ok());
        config.host_request_stdin=false;
        for command in [crate::config::CommandMode::Server,crate::config::CommandMode::Sessions,
            crate::config::CommandMode::SessionShow("fixture".into()),
            crate::config::CommandMode::SessionDelete("fixture".into())] {
            config.command=command;
            assert!(crate::validate_agent_host_invocation(EntryIdentity::AgentHost,&config).is_ok());
        }
    }
}
