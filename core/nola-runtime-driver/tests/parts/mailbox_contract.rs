// SPDX-License-Identifier: Apache-2.0
// Included by opaque_protocol; exercises the actual shared committed pump.
use super::*;
use nola_runtime_driver::mailbox::bounded_host_mailbox;
use std::time::Duration;

fn reply(
    request: &HostRequestEnvelope<Request>,
    sequence: u64,
    payload: Response,
) -> HostResponseEnvelope<Response> {
    HostResponseEnvelope {
        attempt_id: request.attempt_id.clone(),
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        request_id: request.request_id.clone(),
        effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        delivery: request.delivery.clone(),
        sequence,
        payload,
    }
}
fn fixture_request() -> HostRequestEnvelope<Request> {
    let mut app = Application {
        claimed: false,
        reject_input: None,
        accepted_inputs: vec![],
        effects: Arc::new(Mutex::new(vec![])),
    };
    app.claim_committed_requests(RuntimeEffectDeliveryClaim {
        worker_generation: "worker".into(),
        lease_owner: "owner".into(),
        claimed_at_ms: 1,
        lease_expires_at_ms: i64::MAX,
        limit: 1,
    })
    .unwrap()
    .remove(0)
}
#[test]
fn mailbox_physical_execution_waits_for_application_commit_ack() {
    let (host, mailbox) = bounded_host_mailbox::<OpaqueProtocol>(1).unwrap();
    let effects = Arc::new(Mutex::new(vec![]));
    let native_effects = effects.clone();
    let native = std::thread::spawn(move || {
        let (ticket, request) = mailbox.poll(Duration::from_secs(5)).unwrap().unwrap();
        assert!(
            mailbox.poll(Duration::ZERO).unwrap().is_none(),
            "polled delivery must not repeat"
        );
        mailbox
            .respond(ticket, reply(&request, 1, Response::Started))
            .unwrap();
        native_effects.lock().unwrap().push("physical-after-commit");
        mailbox
            .respond(ticket, reply(&request, 2, Response::Finished))
            .unwrap();
    });
    let mut app = Application {
        claimed: false,
        reject_input: None,
        accepted_inputs: vec![],
        effects: effects.clone(),
    };
    test_driver(host).execute(&mut app, &mut |_| {}).unwrap();
    native.join().unwrap();
    let events = effects.lock().unwrap();
    assert!(
        events.iter().position(|e| *e == "committed-start").unwrap()
            < events
                .iter()
                .position(|e| *e == "physical-after-commit")
                .unwrap()
    );
}
#[test]
fn closing_mailbox_unblocks_pending_commit_ack_without_claiming_stop() {
    let (mut host, mailbox) = bounded_host_mailbox::<OpaqueProtocol>(1).unwrap();
    let (entered_tx, entered) = std::sync::mpsc::sync_channel(1);
    let (release, held) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        host.dispatch(fixture_request(), &mut |_| {
            entered_tx.send(()).unwrap();
            held.recv().unwrap();
            Ok(())
        })
    });
    let (ticket, request) = mailbox.poll(Duration::from_secs(5)).unwrap().unwrap();
    let endpoint = mailbox.clone();
    let (done_tx, done) = std::sync::mpsc::sync_channel(1);
    let native = std::thread::spawn(move || {
        done_tx
            .send(endpoint.respond(ticket, reply(&request, 1, Response::Started)))
            .unwrap();
    });
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(done.try_recv().is_err(), "enqueue is not a commit ACK");
    mailbox.close();
    assert!(done
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap_err()
        .contains("unconfirmed"));
    release.send(()).unwrap();
    native.join().unwrap();
    assert!(worker.join().unwrap().is_err());
}
#[test]
fn foreign_ticket_response_never_reaches_application() {
    let (mut host, mailbox) = bounded_host_mailbox::<OpaqueProtocol>(1).unwrap();
    let worker = std::thread::spawn(move || {
        host.dispatch(fixture_request(), &mut |_| {
            panic!("foreign response reached Application")
        })
    });
    let (ticket, request) = mailbox.poll(Duration::from_secs(5)).unwrap().unwrap();
    let mut foreign = reply(&request, 1, Response::Started);
    foreign.request_id = "other-request".into();
    assert!(mailbox
        .respond(ticket, foreign)
        .unwrap_err()
        .contains("another mailbox request"));
    assert!(worker.join().unwrap().is_err());
}
#[test]
fn rejected_start_never_grants_physical_execution() {
    let (mut host, mailbox) = bounded_host_mailbox::<OpaqueProtocol>(1).unwrap();
    let worker = std::thread::spawn(move || {
        host.dispatch(
            fixture_request(),
            &mut |_| Err("Core rejected start".into()),
        )
    });
    let (ticket, request) = mailbox.poll(Duration::from_secs(5)).unwrap().unwrap();
    assert_eq!(
        mailbox
            .respond(ticket, reply(&request, 1, Response::Started))
            .unwrap_err(),
        "Core rejected start"
    );
    assert!(worker.join().unwrap().is_err());
}

#[test]
fn delivery_abort_is_effect_scoped_and_never_acknowledges_a_response_or_stop() {
    let (mut host, mailbox) = bounded_host_mailbox::<OpaqueProtocol>(2).unwrap();
    let worker = std::thread::spawn(move || {
        let first = host.dispatch(fixture_request(), &mut |_| panic!("abort must not forge a response"));
        assert!(first.unwrap_err().contains("stop remains unconfirmed"));
        let mut next = fixture_request(); next.effect_id.0 = "healthy-sibling".into();
        host.dispatch(next, &mut |_| Ok(()))
    });
    let (ticket, request) = mailbox.poll(Duration::from_secs(5)).unwrap().unwrap();
    assert!(!mailbox.abort_delivery("foreign-effect").unwrap());
    assert!(mailbox.abort_delivery(&request.effect_id.0).unwrap());
    assert!(mailbox.respond(ticket, reply(&request, 1, Response::Started)).is_err());
    let (next_ticket, next) = mailbox.poll(Duration::from_secs(5)).unwrap().unwrap();
    assert_eq!(next.effect_id.0, "healthy-sibling");
    assert!(mailbox.respond(ticket, reply(&request, 2, Response::Finished)).is_err(), "an aborted or unknown ticket never gets an ACK");
    mailbox.respond(next_ticket, reply(&next, 1, Response::Started)).unwrap();
    mailbox.respond(next_ticket, reply(&next, 2, Response::Finished)).unwrap();
    worker.join().unwrap().unwrap();
}

#[test]
fn busy_delivery_abort_waits_for_the_real_application_ack_and_leaves_mailbox_open() {
    let (mut host, mailbox) = bounded_host_mailbox::<OpaqueProtocol>(1).unwrap();
    let (entered_tx, entered) = std::sync::mpsc::sync_channel(1);
    let (release, held) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || host.dispatch(fixture_request(), &mut |_| {
        entered_tx.send(()).unwrap(); held.recv().unwrap(); Ok(())
    }));
    let (ticket, request) = mailbox.poll(Duration::from_secs(5)).unwrap().unwrap();
    let endpoint = mailbox.clone(); let effect = request.effect_id.0.clone();
    let (finished, observed) = std::sync::mpsc::sync_channel(1);
    let native = std::thread::spawn(move || finished.send(endpoint.respond(ticket, reply(&request, 1, Response::Started))).unwrap());
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(mailbox.abort_delivery(&effect).unwrap());
    assert!(observed.try_recv().is_err(), "abort is not a synthetic commit ACK");
    release.send(()).unwrap();
    observed.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
    native.join().unwrap();
    assert!(worker.join().unwrap().unwrap_err().contains("stop remains unconfirmed"));
    assert!(mailbox.poll(Duration::ZERO).unwrap().is_none(), "effect abort does not close the owner mailbox");
}
