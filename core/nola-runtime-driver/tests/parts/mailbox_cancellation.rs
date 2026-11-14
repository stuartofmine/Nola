// SPDX-License-Identifier: Apache-2.0
//! Real Application cancellation drained through the shared mailbox driver.
use super::*;
use nola_agent_engine::agent::host::AgentRuntimeProtocol;
use nola_runtime_driver::mailbox::bounded_host_mailbox;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

struct CancelAfterSampling<'a> {
    inner: ApplicationPort<'a>,
    physical_stop: Arc<AtomicBool>,
    assistant: String,
    cancel_accepted: bool,
}
impl CommittedRuntimeApplicationPort for CancelAfterSampling<'_> {
    type Protocol = AgentRuntimeProtocol;
    fn load_committed_requests(&mut self)->Result<Vec<HostRequestEnvelope>,String>{self.inner.load_committed_requests()}
    fn claim_committed_requests(&mut self,claim:RuntimeEffectDeliveryClaim)->Result<Vec<HostRequestEnvelope>,String>{self.inner.claim_committed_requests(claim)}
    fn renew_effect_delivery(&mut self,renewal:RuntimeEffectDeliveryRenewal)->Result<EffectDeliveryLease,String>{self.inner.renew_effect_delivery(renewal)}
    fn accept_boundary_input(&mut self,input:nola_agent_engine::agent::host::AgentBoundaryInput)->Result<CommittedRuntimeAcceptance,String>{self.inner.accept_boundary_input(input)}
    fn accept_raw_host_response(&mut self,value:HostResponseEnvelope)->Result<CommittedRuntimeAcceptance,String>{
        let sampling=matches!(&value.payload,HostResponse::ModelAttemptStarted{..});
        let confirmed=matches!(&value.payload,HostResponse::CancellationConfirmed{..});
        if confirmed {assert!(self.physical_stop.load(Ordering::SeqCst),"a transport abort cannot serve as physical stop evidence");}
        let accepted=self.inner.accept_raw_host_response(value)?;
        if sampling {
            assert!(self.inner.store.local_assistant_run_terminal_seal("committed-runtime",&self.inner.run_id).unwrap().is_none());
            nola_application::request_run_cancellation(self.inner.store,nola_application::RequestRunCancellation{
                conversation_id:"committed-runtime".into(),expected_run_id:self.inner.run_id.clone(),
                assistant_message_id:self.assistant.clone(),cancellation_id:"mailbox-real-stop".into(),reason:None,requested_at_ms:self.inner.now_ms+1})?;
        }
        self.cancel_accepted|=confirmed;
        Ok(accepted)
    }
}
fn mailbox_cancellation(busy:bool){
    let directory=tempdir().unwrap();let mut store=NolaStore::open(directory.path().join("mailbox-stop.db")).unwrap();
    let (run,digest)=begin_silent_model_run(&mut store,if busy{"busy"}else{"idle"});
    let assistant=store.active_run("committed-runtime").unwrap().unwrap().assistant_message_id.unwrap();
    // The persisted assistant identity may differ from the submission/Run ID.
    // Resolve it exactly rather than fabricating a cancellation witness.
    let (host,mailbox)=bounded_host_mailbox::<AgentRuntimeProtocol>(2).unwrap();
    let abort_mailbox=mailbox.clone();let endpoint=mailbox.clone();
    let physical_stop=Arc::new(AtomicBool::new(false));let native_stop=physical_stop.clone();
    let native=std::thread::spawn(move||{
        let (ticket,request)=endpoint.poll(Duration::from_secs(5)).unwrap().unwrap();
        assert!(matches!(request.payload,HostRequest::ModelStream{..}));
        endpoint.respond(ticket,response(&request,1,HostResponse::ExecutionStarted)).unwrap();
        endpoint.respond(ticket,response(&request,2,HostResponse::ModelAttemptStarted{provider:"fixture-provider".into(),model:"fixture-model".into(),reason:"initial".into()})).unwrap();
        if busy {
            let part=match &request.payload{HostRequest::ModelStream{text_part_id,..}=>text_part_id.clone(),_=>unreachable!()};
            let mut sequence=3;
            loop {
                if endpoint.respond(ticket,response(&request,sequence,HostResponse::ModelDelta{channel:ModelDeltaChannel::Text,part_id:part.clone(),delta:"busy".into()})).is_err(){break;}
                sequence+=1;
            }
        }
        let next=endpoint.poll(Duration::from_secs(5)).unwrap();
        if next.is_none(){endpoint.close();panic!("committed Cancel was starved behind the original model delivery");}
        let (cancel_ticket,cancel)=next.unwrap();
        let HostRequest::Cancel{target_effect_id,..}=&cancel.payload else{endpoint.close();panic!("unexpected physical request")};
        assert_eq!(target_effect_id,&request.effect_id);
        endpoint.respond(cancel_ticket,response(&cancel,1,HostResponse::ExecutionStarted)).unwrap();
        native_stop.store(true,Ordering::SeqCst); // concrete test Host stop/join boundary
        endpoint.respond(cancel_ticket,response(&cancel,2,HostResponse::CancellationConfirmed{target_effect_id:target_effect_id.clone()})).unwrap();
    });
    // Preserve the actual stored assistant ID for the real cancellation command.
    let mut port=CancelAfterSampling{inner:ApplicationPort{store:&mut store,invocation_config_digest:digest,run_id:run.clone(),now_ms:2000},physical_stop,assistant,cancel_accepted:false};
    let mut driver=CommittedRuntimeDriver::with_delivery_identity_and_timing(host,"mailbox-worker","mailbox-owner",RuntimeEffectDeliveryTiming{
        lease_duration:Duration::from_secs(30),heartbeat_cadence:Duration::from_millis(100),
    }).with_delivery_abort(Arc::new(move|effect|{abort_mailbox.abort_delivery(effect).unwrap();}));
    driver.execute(&mut port,&mut |_|{}).unwrap();native.join().unwrap();
    assert!(port.cancel_accepted);drop(port);
    let seal=store.local_assistant_run_terminal_seal("committed-runtime",&run).unwrap().unwrap();
    let nola_domain::run_result::RunResultValue::Cancelled{unknown_effect_refs,..}=seal.result.value else{panic!("transport abort is not a successful answer or a failed Run")};
    assert!(unknown_effect_refs.is_empty(),"the native confirmation must settle the exact physical effect");
    assert!(load_committed_host_requests(&store,&run).unwrap().requests.is_empty());
}
#[test]fn idle_mailbox_model_drains_real_cancel_only_after_native_stop(){mailbox_cancellation(false)}
#[test]fn busy_mailbox_model_drains_real_cancel_only_after_native_stop(){mailbox_cancellation(true)}
