// SPDX-License-Identifier: Apache-2.0
// Source-owned heads share the service's commit wakeups, including child writes
// to a parent's Task metadata. No child journal is forwarded to a parent.
struct JournalNotificationStream {
    identity: nola_domain::assistant_journal::AssistantJournalIdentity,
    cancellation: nola_service_client::ApplicationServiceRequestCancellation,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for JournalNotificationStream {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}
fn stream_journal_notifications(
    mut after: nola_domain::assistant_journal::AssistantJournalCursor,
    tx: mpsc::Sender<Vec<u8>>, token: Option<String>, source_host_id: String,
) -> Result<JournalNotificationStream, String> {
    let port = crate::application_port::CliApplicationPort::persistent_default()?;
    #[cfg(test)]
    if !matches!(port, crate::application_port::CliApplicationPort::Service(_)) {
        return stream_in_process_fixture_journal(port, after, tx, token, source_host_id);
    }
    let crate::application_port::CliApplicationPort::Service(client) = port else {
        return Err("journal subscription requires the source service".into());
    };
    let cancellation = nola_service_client::ApplicationServiceRequestCancellation::new();
    let identity = after.identity.clone();
    let stop = cancellation.clone();
    let worker = std::thread::Builder::new().name("source-journal-wait".into()).spawn(move || {
        while !stop.is_cancelled() {
            match client.assistant_journal_wait(&nola_protocol::AssistantJournalWaitRequest {
                after: after.clone(), wait_ms: 30_000,
            }, &stop) {
                Ok(head) => {
                    if head.sequence > after.sequence {
                        let event = json!({"type":"assistant_journal_committed_v1",
                            "session_id":head.identity.conversation_id,
                            "source_host_id":source_host_id,"through":head});
                        if tx.send(mobile_sse_event(event, token.as_deref())).is_err() { break; }
                    }
                    after = head;
                }
                Err(error) => {
                    if !stop.is_cancelled() {
                        let _ = tx.send(mobile_sse_event(json!({"type":"error",
                            "message":format!("Committed journal observation unavailable: {error}")}), token.as_deref()));
                    }
                    break;
                }
            }
        }
    }).map_err(|error| error.to_string())?;
    Ok(JournalNotificationStream { identity, cancellation, worker: Some(worker) })
}

fn observe_journal_for_stream(
    stream: &mut Option<JournalNotificationStream>, owner: &ObservedWorkerOwner,
    event: &Value, tx: &mpsc::Sender<Vec<u8>>, token: &Option<String>, host_id: &str,
) -> bool {
    if event["type"] != "assistant_journal_committed_v1" { return false; }
    let Ok(through) = serde_json::from_value::<nola_domain::assistant_journal::AssistantJournalCursor>(event["through"].clone()) else { return false; };
    if let Some(stream) = stream {
        // The live follower publishes this message's latest committed head.
        // Other segments and failed followers still need direct delivery.
        return event.get("trace").is_none() && stream.identity == through.identity
            && stream.worker.as_ref().is_some_and(|worker| !worker.is_finished());
    }
    if through.identity.conversation_id != owner.conversation_id || through.identity.run_id != owner.run_id
        || through.identity.assistant_message_id != owner.assistant_message_id { return false; }
    match stream_journal_notifications(through, tx.clone(), token.clone(), host_id.into()) {
        Ok(subscription) => *stream = Some(subscription),
        Err(error) => { let _ = tx.send(mobile_sse_event(json!({"type":"error","message":error}), token.as_deref())); }
    }
    false
}

#[cfg(test)]
mod notification_delivery_tests {
    use super::*;

    #[test]
    fn follower_coalesces_only_its_live_message_wakeups() {
        let identity = nola_domain::assistant_journal::AssistantJournalIdentity {
            source_store_epoch: "epoch".into(), conversation_id: "conversation".into(),
            run_id: "run".into(), assistant_message_id: "assistant".into(),
        };
        let cursor = nola_domain::assistant_journal::AssistantJournalCursor::initial(identity.clone()).unwrap();
        let (stop, stopped) = mpsc::channel();
        let worker = std::thread::spawn(move || { let _ = stopped.recv_timeout(StdDuration::from_secs(2)); });
        let mut subscription = Some(JournalNotificationStream { identity, worker: Some(worker),
            cancellation: nola_service_client::ApplicationServiceRequestCancellation::new() });
        let owner = ObservedWorkerOwner { conversation_id: "conversation".into(), run_id: "run".into(),
            assistant_message_id: "assistant".into(), client_id: "client".into(), generation: 1 };
        let (tx, _rx) = mpsc::channel();
        let event = json!({"type":"assistant_journal_committed_v1", "through":cursor});
        // A bounded duplicate-hint burst must not multiply the follower's
        // queue; traced controls, other owners and terminal events stay direct.
        for _ in 0..1_000 {
            assert!(observe_journal_for_stream(&mut subscription, &owner, &event, &tx, &None, "host"));
        }
        let mut with_trace = event.clone();
        with_trace["trace"] = json!({"sequence":1});
        assert!(!observe_journal_for_stream(&mut subscription, &owner, &with_trace, &tx, &None, "host"));
        let mut other_message = event.clone();
        other_message["through"]["identity"]["assistant_message_id"] = "next-assistant".into();
        assert!(!observe_journal_for_stream(&mut subscription, &owner, &other_message, &tx, &None, "host"));
        let terminal = json!({"type":"assistant_run_terminal_committed_v1"});
        assert!(!observe_journal_for_stream(&mut subscription, &owner, &terminal, &tx, &None, "host"));
        stop.send(()).unwrap();
        subscription.as_mut().unwrap().worker.take().unwrap().join().unwrap();
        assert!(!observe_journal_for_stream(&mut subscription, &owner, &event, &tx, &None, "host"));
    }
}

// Unit Host fixtures intentionally compose Application in-process. Observe its
// actual committed head here; production always uses the Service wait protocol,
// whose wakeup and cancellation contracts have separate IPC tests.
#[cfg(test)]
fn stream_in_process_fixture_journal(
    mut port: crate::application_port::CliApplicationPort,
    mut after: nola_domain::assistant_journal::AssistantJournalCursor,
    tx: mpsc::Sender<Vec<u8>>, token: Option<String>, source_host_id: String,
) -> Result<JournalNotificationStream, String> {
    let cancellation = nola_service_client::ApplicationServiceRequestCancellation::new();
    let stop = cancellation.clone();
    let identity = after.identity.clone();
    let worker = std::thread::Builder::new().name("fixture-source-journal".into()).spawn(move || {
        while !stop.is_cancelled() {
            let head: Result<nola_domain::assistant_journal::AssistantJournalCursor, String> = port.execute(
                nola_application::ApplicationCommand::LocalAssistantJournalHead { identity: after.identity.clone() });
            let Ok(head) = head else { break };
            if head.sequence > after.sequence {
                let event = json!({"type":"assistant_journal_committed_v1",
                    "session_id":head.identity.conversation_id,"source_host_id":source_host_id,"through":head});
                if tx.send(mobile_sse_event(event, token.as_deref())).is_err() { break; }
            }
            after = head;
            std::thread::sleep(StdDuration::from_millis(10));
        }
    }).map_err(|error| error.to_string())?;
    Ok(JournalNotificationStream { identity, cancellation, worker: Some(worker) })
}
