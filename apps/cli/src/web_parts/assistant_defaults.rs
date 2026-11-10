fn assistant_retrieval_default_response(enabled: Option<bool>) -> Response<Box<dyn Read + Send>> {
    let result = (|| -> Result<Value, String> {
        let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
        if let Some(enabled) = enabled {
            crate::assistant_retrieval_preferences::save_default(&mut app, enabled)?;
        }
        Ok(json!({"enabled":crate::assistant_retrieval_preferences::read_default(&mut app)?}))
    })();
    match result {
        Ok(snapshot) => json_response(StatusCode(200), snapshot),
        Err(_) => json_response(StatusCode(if enabled.is_some() { 409 } else { 500 }), json!({"code":"retrieval_default_unconfirmed"})),
    }
}

fn assistant_memory_default_response(enabled: Option<bool>) -> Response<Box<dyn Read + Send>> {
    let result = (|| -> Result<Value, String> {
        let host_allowed = crate::host_memory_admission::resolve_host_memory_policy(None)?.long_term_enabled;
        let path = crate::platform_paths::application_database_path()?;
        let mut app = crate::application_port::CliApplicationPort::persistent(&path)?;
        if let Some(enabled) = enabled {
            crate::assistant_memory_preferences::save_default(&mut app, enabled, host_allowed)?;
        }
        let actual = crate::assistant_memory_preferences::read_default(&mut app)?;
        Ok(json!({"enabled":actual,"hostAllowed":host_allowed}))
    })();
    match result {
        Ok(snapshot) => json_response(StatusCode(200), snapshot),
        Err(_) => json_response(StatusCode(if enabled.is_some() { 409 } else { 500 }), json!({"code":"memory_default_unconfirmed"})),
    }
}
