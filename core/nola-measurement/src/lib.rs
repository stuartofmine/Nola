// SPDX-License-Identifier: Apache-2.0
//! Lossy diagnostic observations. Never a billing, delivery or permission ledger.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{cell::RefCell, io::Write, marker::PhantomData, rc::Rc,
    sync::{atomic::{AtomicU64, Ordering}, mpsc, OnceLock}, time::Instant};

const CAPACITY: usize = 1024;
static RECORDER: OnceLock<Option<Recorder>> = OnceLock::new();
thread_local! { static CONTEXT: RefCell<Context> = RefCell::new(Context::default()); }

#[derive(Clone, Debug, Default, Serialize)]
pub struct Context {
    pub command: Option<String>,
    pub run: Option<String>,
    pub command_kind: Option<CommandKind>,
    pub session: Option<String>,
    pub effect: Option<String>,
    pub request: Option<String>,
    pub attempt: Option<String>,
    pub parent_span: Option<u64>,
    pub physical_request: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all="snake_case")]
pub enum CommandKind { BeginTurn, ActiveSubmission, IdleSubmission, QueuedTurn, TaskTurn, AutomationTurn, Other }
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all="snake_case")]
pub enum Stage { ProviderSampling, HttpHeaders, AdapterFirstText, AdapterFirstReasoning,
    HostTool, HostCompaction, StoreCommit, StoreCommittedHostResponse,
    CommandExecution, CommandReceiptCommitted }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all="snake_case")]
pub enum Outcome { Succeeded, Failed, Cancelled, Rejected, Incomplete }
#[derive(Serialize)]
struct Event {
    schema_version: u8,
    build_digest: Option<String>,
    process_id: u32,
    clock_domain: String,
    observation_id: u64,
    span_id: u64,
    monotonic_ns: String,
    elapsed_ns: Option<String>,
    stage: Stage,
    event: &'static str,
    outcome: Option<Outcome>,
    context: Context,
    dropped_total: u64,
}
struct Recorder {
    sender: mpsc::SyncSender<Event>, epoch: Instant, domain: String,
    sequence: AtomicU64, dropped: AtomicU64,
    build_digest: Option<String>,
}
fn recorder() -> Option<&'static Recorder> {
    RECORDER.get_or_init(|| {
        // No channel, writer thread, clock reads or ID allocation when disabled.
        let path=std::env::var_os("NOLA_MEASUREMENT_FILE")?;
        let (sender, receiver)=mpsc::sync_channel::<Event>(CAPACITY);
        std::thread::Builder::new().name("nola-measurement".into()).spawn(move || {
            // Opening a slow filesystem is I/O too: even initial open belongs
            // on the consumer, never the first model-request thread.
            let mut options=std::fs::OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)] {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let Ok(file)=options.open(path) else { return; };
            write_events(file, receiver);
        }).ok()?;
        Some(Recorder {sender,epoch:Instant::now(),domain:uuid::Uuid::new_v4().to_string(),
            sequence:AtomicU64::new(1),dropped:AtomicU64::new(0),
            build_digest:std::env::var("NOLA_MEASUREMENT_BUILD_DIGEST").ok()
                .filter(|v|v.len()==64 && v.bytes().all(|b|b.is_ascii_hexdigit()))})
    }).as_ref()
}
fn write_events(writer:impl Write, receiver:mpsc::Receiver<Event>) {
    let mut writer=std::io::BufWriter::new(writer);
    while let Ok(event)=receiver.recv() {
        // Serialization and disk I/O never run on a business/first-token thread.
        if serde_json::to_writer(&mut writer,&event).is_err()
            || writer.write_all(b"\n").is_err() || writer.flush().is_err() { break; }
    }
}
/// Whether the recorder was configured, not proof that the diagnostic sink
/// opened successfully or that any particular observation reached disk.
pub fn enabled() -> bool { recorder().is_some() }
/// Receipt identity is a tuple, not a delimiter-joined string. Experiment
/// manifests use this same domain-separated, length-framed hash.
pub fn command_identity(client: &str, request: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"nola.command.v1\0");
    for part in [client, request] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}
pub fn command_context(client: &str, request: &str, session: Option<&str>,
    run: Option<&str>, kind: CommandKind) -> Context {
    if !enabled() { return Context::default(); }
    Context { command: Some(command_identity(client, request)), run: run.map(hashed),
        session: session.map(hashed), command_kind: Some(kind), ..Default::default() }
}
pub fn with_run(mut context: Context, run: Option<&str>) -> Context {
    if enabled() { if let Some(run) = run { context.run = Some(hashed(run)); } }
    context
}
pub fn dropped() { if let Some(r)=recorder(){r.dropped.fetch_add(1,Ordering::Relaxed);} }
pub fn store_context(effect:&str,request:&str)->Context {
    if !enabled(){return Context::default()}
    Context{effect:Some(hashed(effect)),request:Some(hashed(request)),..Default::default()}
}
pub fn with_attempt(mut context:Context,attempt:Option<&str>)->Context {
    if enabled(){context.attempt=attempt.map(hashed);} context
}
pub fn point_outcome(stage:Stage,outcome:Outcome){if let Some(r)=recorder(){emit(stage,"point",next(r),None,Some(outcome),context());}}
fn next(r:&Recorder)->u64 { r.sequence.fetch_add(1,Ordering::Relaxed) }
fn hashed(value:&str)->String { format!("{:x}",Sha256::digest(value.as_bytes())) }
/// Only opaque identities are accepted; raw identities are hashed before recording.
pub fn host_context(session:&str,effect:&str,request:&str)->Context {
    if !enabled() {return Context::default()}
    Context {session:Some(hashed(session)),effect:Some(hashed(effect)),request:Some(hashed(request)),
        ..Default::default()}
}
pub fn context()->Context { CONTEXT.with(|c| c.borrow().clone()) }
pub fn set_attempt(attempt:&str) {if enabled(){CONTEXT.with(|c|c.borrow_mut().attempt=Some(hashed(attempt)));}}
pub fn inherit_attempt(value:&Context) {if enabled(){CONTEXT.with(|c|c.borrow_mut().attempt=value.attempt.clone());}}
pub struct Scope { previous:Context, _thread:PhantomData<Rc<()>> }
pub fn enter(value:Context)->Scope {
    Scope {previous:CONTEXT.with(|c|c.replace(value)),_thread:PhantomData}
}
impl Drop for Scope { fn drop(&mut self) { CONTEXT.with(|c|c.replace(std::mem::take(&mut self.previous))); } }
fn emit(stage:Stage,event:&'static str,span_id:u64,elapsed_ns:Option<String>,outcome:Option<Outcome>,context:Context) {
    let Some(r)=recorder() else{return};
    let item=Event {schema_version:1,build_digest:r.build_digest.clone(),process_id:std::process::id(),clock_domain:r.domain.clone(),observation_id:next(r),span_id,
        monotonic_ns:r.epoch.elapsed().as_nanos().to_string(),elapsed_ns,stage,event,outcome,context,
        dropped_total:r.dropped.load(Ordering::Relaxed)};
    publish(r,item);
}
fn publish(r:&Recorder,item:Event){if r.sender.try_send(item).is_err(){r.dropped.fetch_add(1,Ordering::Relaxed);}}
pub struct Span {stage:Stage,id:u64,started:Option<Instant>,context:Context,outcome:Outcome}
impl Span {
    pub fn start(stage:Stage)->Self {
        let r=recorder();
        let mut context=if r.is_some(){context()}else{Context::default()};
        if matches!(stage,Stage::ProviderSampling){context.physical_request=None;}
        let span=Self{stage,id:r.map(next).unwrap_or(0),started:r.map(|_|Instant::now()),
            context,outcome:Outcome::Incomplete};
        if r.is_some(){emit(stage,"begin",span.id,None,None,span.context.clone());}
        span
    }
    /// Each actual HTTP send, including DNS/non-stream fallback, gets a fresh ID.
    pub fn physical_send()->Self {
        if let Some(r)=recorder(){CONTEXT.with(|c|c.borrow_mut().physical_request=Some(next(r)));}
        Self::start(Stage::HttpHeaders)
    }
    pub fn child_context(&self)->Context {let mut c=self.context.clone();c.parent_span=Some(self.id);c}
    pub fn finish(mut self,outcome:Outcome){self.outcome=outcome;}
}
impl Drop for Span {fn drop(&mut self){if let Some(start)=self.started {
    emit(self.stage,"end",self.id,Some(start.elapsed().as_nanos().to_string()),Some(self.outcome),self.context.clone());
}}}
pub fn point(stage:Stage){if let Some(r)=recorder(){emit(stage,"point",next(r),None,None,context());}}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn scope_restores_and_propagates_explicitly() {
        let original=context();
        {let _scope=enter(Context{parent_span:Some(42),..Default::default()});
         let inherited=context();
         std::thread::spawn(move || {let _scope=enter(inherited);assert_eq!(context().parent_span,Some(42));}).join().unwrap();}
        assert_eq!(context().parent_span,original.parent_span);
    }
    #[test] fn outcome_does_not_infer_cancellation() {
        assert_eq!(Span::start(Stage::HostTool).outcome,Outcome::Incomplete);
    }
    #[test] fn identifiers_do_not_disclose_input() {
        assert_eq!(hashed("same"),hashed("same"));assert!(!hashed("secret").contains("secret"));
    }
    #[test] fn command_tuple_hash_is_unambiguous_and_stable() {
        assert_eq!(command_identity("client", "request"), command_identity("client", "request"));
        assert_ne!(command_identity("a", "bc"), command_identity("ab", "c"));
        assert_ne!(command_identity("a:b", "c"), command_identity("a", "b:c"));
        assert_eq!(command_identity("客户端", "请求").len(), 64);
    }
    #[test] fn failed_writer_disconnects_diagnostic_sink() {
        struct FailedWriter;
        impl Write for FailedWriter {
            fn write(&mut self, _: &[u8])->std::io::Result<usize> {
                Err(std::io::Error::other("diagnostic fixture failure"))
            }
            fn flush(&mut self)->std::io::Result<()> { Ok(()) }
        }
        let (sender,receiver)=mpsc::sync_channel(1);
        assert!(sender.try_send(Event{schema_version:1,build_digest:None,process_id:1,
            clock_domain:"fixture".into(),observation_id:1,span_id:1,
            monotonic_ns:"1".into(),elapsed_ns:None,stage:Stage::StoreCommit,
            event:"point",outcome:None,context:Context::default(),dropped_total:0}).is_ok());
        write_events(FailedWriter,receiver);
        // The producer can detect a disconnected sink without waiting for I/O.
        assert!(matches!(sender.try_send(Event{schema_version:1,build_digest:None,
            process_id:1,clock_domain:"fixture".into(),observation_id:2,span_id:2,
            monotonic_ns:"2".into(),elapsed_ns:None,stage:Stage::StoreCommit,
            event:"point",outcome:None,context:Context::default(),dropped_total:0}),
            Err(mpsc::TrySendError::Disconnected(_))));
    }
    #[test] fn bounded_sink_drops_without_blocking_or_changing_outcome() {
        let (sender,receiver)=mpsc::sync_channel(1);
        let recorder=Recorder{sender,epoch:Instant::now(),domain:"fixture".into(),
            sequence:AtomicU64::new(1),dropped:AtomicU64::new(0),build_digest:None};
        let event=|outcome|Event{schema_version:1,build_digest:None,process_id:1,clock_domain:"fixture".into(),
            observation_id:1,span_id:1,monotonic_ns:"1".into(),elapsed_ns:Some("1".into()),
            stage:Stage::HttpHeaders,event:"end",outcome:Some(outcome),context:Context::default(),dropped_total:0};
        publish(&recorder,event(Outcome::Cancelled));
        publish(&recorder,event(Outcome::Failed));
        assert_eq!(recorder.dropped.load(Ordering::Relaxed),1);
        assert_eq!(receiver.recv().unwrap().outcome,Some(Outcome::Cancelled));
        drop(receiver);
        publish(&recorder,event(Outcome::Succeeded));
        assert_eq!(recorder.dropped.load(Ordering::Relaxed),2);
    }
}
