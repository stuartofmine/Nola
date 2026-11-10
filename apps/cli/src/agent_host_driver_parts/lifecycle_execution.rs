// SPDX-License-Identifier: Apache-2.0
// Shared lifecycle executor; owns no model or general tool driver.
#[derive(Clone, Default)]
struct LifecycleHookExecutor {
    lifecycle_hook_rules: Vec<nola_core::lifecycle_hooks::LifecycleHookRule>,
    lifecycle_hook_registry_revision_sha256: Option<String>,
    lifecycle_hook_handler: Option<std::sync::Arc<std::sync::Mutex<LifecycleHookHandler>>>,
    lifecycle_hook_capabilities: Vec<nola_core::lifecycle_hooks::LifecycleHookRequiredCapability>,
    current_turn_lifecycle_event_ids: HashSet<String>,
    lifecycle_session_event_ids: HashSet<String>,
}
impl LifecycleHookExecutor {
    fn run_lifecycle_hooks(
        &mut self,
        request: &HostRequestEnvelope,
        sequence: &mut u64,
        event_name: nola_core::lifecycle_hooks::LifecycleHookEventName,
        event_id: Option<&str>,
        tool_name: Option<&str>,
        tool_call_id: Option<&str>,
        once_per_turn: bool,
        require_success: bool,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.lifecycle_hook_rules.is_empty() {
            return Ok(());
        }
        let event_name_id = match event_name {
            nola_core::lifecycle_hooks::LifecycleHookEventName::SessionStart => "session_start",
            nola_core::lifecycle_hooks::LifecycleHookEventName::UserPromptSubmit => {
                "user_prompt_submit"
            }
            nola_core::lifecycle_hooks::LifecycleHookEventName::PreToolUse => "pre_tool_use",
            nola_core::lifecycle_hooks::LifecycleHookEventName::PostToolUse => "post_tool_use",
            nola_core::lifecycle_hooks::LifecycleHookEventName::ToolError => "tool_error",
            nola_core::lifecycle_hooks::LifecycleHookEventName::TurnEnd => "turn_end",
            nola_core::lifecycle_hooks::LifecycleHookEventName::SessionEnd => "session_end",
        };
        if once_per_turn {
            let inserted = match event_name {
                nola_core::lifecycle_hooks::LifecycleHookEventName::SessionStart
                | nola_core::lifecycle_hooks::LifecycleHookEventName::SessionEnd => self
                    .lifecycle_session_event_ids
                    .insert(format!("{}:{event_name_id}", request.session_id)),
                _ => self.current_turn_lifecycle_event_ids.insert(format!(
                    "{}:{event_name_id}",
                    request.turn_id.as_deref().unwrap_or("session")
                )),
            };
            if !inserted {
                if require_success {
                    return Err("session-end hook already attempted; durable recovery is required".into());
                }
                return Ok(());
            }
        }
        let event = nola_core::lifecycle_hooks::LifecycleHookEvent {
            id: event_id
                .map(str::to_string)
                .unwrap_or_else(|| format!("event.{}.{}", request.effect_id.0, event_name_id)),
            name: event_name,
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            tool_name: tool_name.map(str::to_string),
            tool_call_id: tool_call_id.map(str::to_string),
        };
        let Ok(plan) = nola_core::lifecycle_hooks::plan_lifecycle_hooks(
            &nola_core::lifecycle_hooks::LifecycleHookPlanInput {
                schema_version: nola_core::lifecycle_hooks::LIFECYCLE_HOOK_SCHEMA_VERSION,
                event: Some(event.clone()),
                rules: self.lifecycle_hook_rules.clone(),
            },
        ) else {
            if require_success {
                return Err("session-end hook plan is invalid".into());
            }
            return Ok(());
        };
        if require_success && plan.rejected_rules.iter().any(|rule| matches!(rule.reason,
            nola_core::lifecycle_hooks::LifecycleHookRejectionReason::InvalidRule
                | nola_core::lifecycle_hooks::LifecycleHookRejectionReason::DuplicateRule)) {
            return Err("session-end hook registry contains invalid or duplicate rules".into());
        }
        for invocation in plan.invocations {
            let running = nola_core::lifecycle_hooks::project_lifecycle_hook_execution(
                &nola_core::lifecycle_hooks::LifecycleHookExecutionInput {
                    schema_version: nola_core::lifecycle_hooks::LIFECYCLE_HOOK_SCHEMA_VERSION,
                    registry_revision_sha256: self
                        .lifecycle_hook_registry_revision_sha256
                        .clone()
                        .unwrap_or_default(),
                    invocation: invocation.clone(),
                    status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Running,
                    duration_ms: None,
                    output_preview: None,
                    output_truncated: false,
                    error_code: None,
                },
            );
            if require_success && running.is_err() {
                return Err("session-end hook start projection is invalid".into());
            }
            if let Ok(projection) = running {
                CliAgentHostDriver::emit(
                    request,
                    sequence,
                    HostResponse::LifecycleHookUpdated { projection },
                    emit,
                )?;
            }

            let capability_available = invocation.required_capability
                == nola_core::lifecycle_hooks::LifecycleHookRequiredCapability::None
                || self
                    .lifecycle_hook_capabilities
                    .contains(&invocation.required_capability);
            let started = std::time::Instant::now();
            let result = if !capability_available {
                HostLifecycleHookResult {
                    status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                    output_preview: None,
                    output_truncated: false,
                    error_code: Some("hook_capability_unavailable".to_string()),
                }
            } else if invocation.handler.execution_mode
                == nola_core::lifecycle_hooks::LifecycleHookExecutionMode::Async
            {
                HostLifecycleHookResult {
                    status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                    output_preview: None,
                    output_truncated: false,
                    error_code: Some("hook_async_executor_unavailable".to_string()),
                }
            } else if let Some(handler) = self.lifecycle_hook_handler.as_ref().cloned() {
                let invocation_for_handler = invocation.clone();
                let event_for_handler = event.clone();
                let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    let result = handler
                        .lock()
                        .map_err(|_| "hook_handler_lock_failed".to_string())
                        .and_then(|mut handler| {
                            handler(&invocation_for_handler, &event_for_handler)
                        });
                    let _ = result_tx.send(result);
                });
                match result_rx
                    .recv_timeout(std::time::Duration::from_millis(invocation.timeout_ms))
                {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => HostLifecycleHookResult {
                        status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                        output_preview: None,
                        output_truncated: false,
                        error_code: Some("hook_handler_failed".to_string()),
                    },
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => HostLifecycleHookResult {
                        status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::TimedOut,
                        output_preview: None,
                        output_truncated: false,
                        error_code: Some("hook_timed_out".to_string()),
                    },
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        HostLifecycleHookResult {
                            status:
                                nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                            output_preview: None,
                            output_truncated: false,
                            error_code: Some("hook_handler_disconnected".to_string()),
                        }
                    }
                }
            } else {
                HostLifecycleHookResult {
                    status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                    output_preview: None,
                    output_truncated: false,
                    error_code: Some("hook_handler_unavailable".to_string()),
                }
            };
            let duration_ms = started.elapsed().as_millis().min(180_000) as u64;
            let timed_out = result.status
                == nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::TimedOut
                || duration_ms > invocation.timeout_ms;
            let succeeded = !timed_out && result.status
                == nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Succeeded;
            let mut preview = result.output_preview;
            let preview_was_truncated = preview
                .as_ref()
                .is_some_and(|value| value.chars().count() > 4_096);
            if preview_was_truncated {
                preview = preview.map(|value| value.chars().take(4_096).collect());
            }
            let terminal = nola_core::lifecycle_hooks::project_lifecycle_hook_execution(
                &nola_core::lifecycle_hooks::LifecycleHookExecutionInput {
                    schema_version: nola_core::lifecycle_hooks::LIFECYCLE_HOOK_SCHEMA_VERSION,
                    registry_revision_sha256: self
                        .lifecycle_hook_registry_revision_sha256
                        .clone()
                        .unwrap_or_default(),
                    invocation,
                    status: if timed_out {
                        nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::TimedOut
                    } else if result.status
                        == nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Running
                    {
                        nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed
                    } else {
                        result.status
                    },
                    duration_ms: Some(duration_ms),
                    output_preview: preview,
                    output_truncated: result.output_truncated || preview_was_truncated,
                    error_code: if timed_out {
                        Some("hook_timed_out".to_string())
                    } else {
                        result.error_code
                    },
                },
            );
            if require_success && terminal.is_err() {
                return Err("session-end hook terminal projection is invalid; outcome is unconfirmed".into());
            }
            if let Ok(projection) = terminal {
                CliAgentHostDriver::emit(
                    request,
                    sequence,
                    HostResponse::LifecycleHookUpdated { projection },
                    emit,
                )?;
            }
            if require_success && !succeeded {
                return Err(if timed_out {
                    "session-end hook timed out; external execution may still be active"
                } else {
                    "session-end hook did not succeed; session closure is unconfirmed"
                }.into());
            }
        }
        Ok(())
    }


}
#[derive(Default)]
pub(crate) struct LifecycleOnlyDriver {
    executor: LifecycleHookExecutor,
}
impl CliAgentHostDriver {
    pub(crate) fn lifecycle_only_driver(&self) -> LifecycleOnlyDriver {
        LifecycleOnlyDriver { executor: self.lifecycle.clone() }
    }
}
impl AgentHostDriver for LifecycleOnlyDriver {
    type Protocol = nola_core::agent::host::AgentRuntimeProtocol;
    type Error = String;
    fn dispatch(&mut self, request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        let HostRequest::RunLifecycleHook { event } = &request.payload else {
            return Err("lifecycle-only driver refuses non-lifecycle work".into());
        };
        if event.name != nola_core::lifecycle_hooks::LifecycleHookEventName::SessionEnd
            || event.session_id != request.session_id || event.turn_id != request.turn_id {
            return Err("lifecycle event identity mismatch".into());
        }
        let mut sequence = CliAgentHostDriver::response_sequence(&request);
        if request.delivery.is_some() && !request.attempt_started {
            CliAgentHostDriver::emit(&request, &mut sequence, HostResponse::ExecutionStarted, emit)?;
        }
        self.executor.run_lifecycle_hooks(&request, &mut sequence, event.name,
            Some(&event.id), event.tool_name.as_deref(), event.tool_call_id.as_deref(), true, true, emit)?;
        CliAgentHostDriver::emit(&request, &mut sequence, HostResponse::Completed {
            output: serde_json::json!({}), usage: None, full_output_ref: None, artifacts: Vec::new(),
        }, emit)
    }
}

#[cfg(test)]
mod lifecycle_close_failure_tests {
    use super::*;
    use nola_core::lifecycle_hooks::*;

    fn request() -> HostRequestEnvelope {
        serde_json::from_value(serde_json::json!({
            "attempt_started":false,"attempt_id":"close-attempt","request_id":"close-request",
            "effect_id":"close-effect","host_id":"close-host","session_id":"close-session",
            "turn_id":"close-turn","catalog_revision":1,"availability_epoch":1,"created_at_ms":1,
            "payload":{"type":"run_lifecycle_hook","event":{"id":"close-event",
                "name":"session_end","sessionId":"close-session","turnId":"close-turn"}}
        })).unwrap()
    }

    fn executor(status: LifecycleHookExecutionStatus) -> LifecycleHookExecutor {
        LifecycleHookExecutor {
            lifecycle_hook_rules: vec![LifecycleHookRule {
                id:"close-rule".into(), event:LifecycleHookEventName::SessionEnd,
                source:LifecycleHookSource::User, trust:LifecycleHookTrustStatus::Trusted,
                handler:LifecycleHookHandlerReference {id:"close-handler".into(),
                    kind:LifecycleHookHandlerKind::Internal, execution_mode:LifecycleHookExecutionMode::Sync,
                    display_name:None}, enabled:true, matcher:Default::default(), display_order:0, timeout_ms:100,
            }],
            lifecycle_hook_registry_revision_sha256:Some("a".repeat(64)),
            lifecycle_hook_handler:Some(std::sync::Arc::new(std::sync::Mutex::new(Box::new(move |_,_| {
                Ok(HostLifecycleHookResult {status,output_preview:None,output_truncated:false,error_code:None})
            })))),
            ..Default::default()
        }
    }

    #[test]
    fn failed_or_timed_out_session_end_never_emits_completed_or_replays() {
        for status in [LifecycleHookExecutionStatus::Failed, LifecycleHookExecutionStatus::TimedOut] {
            let mut driver = LifecycleOnlyDriver {executor:executor(status)};
            let mut responses=Vec::new();
            assert!(driver.dispatch(request(), &mut |value| {responses.push(value);Ok(())}).is_err());
            assert!(responses.iter().any(|value| matches!(&value.payload,
                HostResponse::LifecycleHookUpdated {projection} if projection.status==status)));
            assert!(!responses.iter().any(|value| matches!(value.payload, HostResponse::Completed {..})));
            let count=responses.len();
            assert!(driver.dispatch(request(), &mut |value| {responses.push(value);Ok(())}).is_err());
            assert_eq!(responses.len(),count,"retry cannot turn prior failure into success");
        }
    }

    #[test]
    fn invalid_session_end_plan_and_registry_do_not_complete() {
        for invalid_event in [true,false] {
            let mut driver=LifecycleOnlyDriver {executor:executor(LifecycleHookExecutionStatus::Succeeded)};
            let mut request=request();
            if invalid_event {
                if let HostRequest::RunLifecycleHook {event}=&mut request.payload {event.id=String::new();}
            } else {driver.executor.lifecycle_hook_rules[0].id=String::new();}
            let mut responses=Vec::new();
            assert!(driver.dispatch(request,&mut |value| {responses.push(value);Ok(())}).is_err());
            assert!(!responses.iter().any(|value| matches!(value.payload,HostResponse::Completed {..})));
        }
    }

    #[test]
    fn ordinary_observation_hook_failure_remains_nonblocking() {
        let mut executor=executor(LifecycleHookExecutionStatus::Failed);
        executor.run_lifecycle_hooks(&request(), &mut 0, LifecycleHookEventName::SessionEnd,
            Some("close-event"),None,None,true,false,&mut |_| Ok(())).unwrap();
    }

    #[test]
    fn still_running_session_end_timeout_never_claims_completion() {
        let (release_tx,release_rx)=std::sync::mpsc::channel();
        let (finished_tx,finished_rx)=std::sync::mpsc::channel();
        let mut executor=executor(LifecycleHookExecutionStatus::Succeeded);
        executor.lifecycle_hook_handler=Some(std::sync::Arc::new(std::sync::Mutex::new(Box::new(move |_,_| {
            release_rx.recv().map_err(|error|error.to_string())?;
            let _=finished_tx.send(());
            Ok(HostLifecycleHookResult {status:LifecycleHookExecutionStatus::Succeeded,
                output_preview:None,output_truncated:false,error_code:None})
        }))));
        let mut driver=LifecycleOnlyDriver {executor};
        let mut responses=Vec::new();
        let result=driver.dispatch(request(),&mut |value| {responses.push(value);Ok(())});
        // Explicitly release/join the fixture handler; no orphan background work.
        release_tx.send(()).unwrap();
        finished_rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
        assert!(result.unwrap_err().contains("external execution may still be active"));
        assert!(!responses.iter().any(|value| matches!(value.payload,HostResponse::Completed {..})));
    }
}

