// SPDX-License-Identifier: Apache-2.0
//! Client request journal only. Application command receipts/FIFO remain the
//! canonical admission and delivery facts. No authorization is stored here.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: usize = 2 * 1024 * 1024;
const SOURCE_OWNER: &str = "local-host-boundary-input-v1";
static RETIREMENT_PENDING: OnceLock<Mutex<HashMap<PathBuf,Intent>>> = OnceLock::new();
fn retirement_pending()-> &'static Mutex<HashMap<PathBuf,Intent>> { RETIREMENT_PENDING.get_or_init(||Mutex::new(HashMap::new())) }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    version: u32,
    database: String,
    source_owner: String,
    session: String,
    request_id: String,
    submission_id: String,
    payload_json: String,
    created_at_ms: i64,
}

pub(crate) enum InputOperation<'a> { NewSubmit(&'a str), ResumePending(&'a str), Inspect }

struct Journal { database: PathBuf, directory: PathBuf, path: PathBuf, session: String, _lock: File }
impl Journal {
    fn open(database: &Path, session: &str) -> Result<Self,String> {
        if session.trim().is_empty() || session.len()>512 || session.chars().any(char::is_control) { return Err("invalid input session".into()); }
        let parent = database.parent().ok_or("database directory missing")?;
        fs::create_dir_all(parent).map_err(|e|e.to_string())?;
        let parent=parent.canonicalize().map_err(|e|e.to_string())?;
        let database=parent.join(database.file_name().ok_or("database filename missing")?);
        let directory=parent.join("client-input-intents");
        let mut builder=fs::DirBuilder::new();
        #[cfg(unix)] {use std::os::unix::fs::DirBuilderExt; builder.mode(0o700);}
        match builder.create(&directory) { Ok(())=>{sync_directory(&parent)?;},Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>{},Err(e)=>return Err(e.to_string()) }
        private_directory(&directory)?;
        // Also finish a previous instance's failed directory publication.
        // Syncing only the intent's directory would not persist that directory
        // entry in its own parent after a crash.
        sync_directory(&parent)?;
        let key=format!("{:x}",Sha256::digest(serde_json::to_vec(&(SOURCE_OWNER,&database,session)).map_err(|e|e.to_string())?));
        let lock=private_file(&directory.join(format!("{key}.lock")),false)?;
        if !lock.metadata().map_err(|e|e.to_string())?.is_file() {return Err("input lock must be a regular file".into());}
        lock.lock().map_err(|e|e.to_string())?;
        Ok(Self {database,directory:directory.clone(),path:directory.join(format!("{key}.json")),session:session.into(),_lock:lock})
    }
    fn read(&self) -> Result<Option<Intent>,String> {
        let mut options=OpenOptions::new(); options.read(true).write(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW);}
        let mut file=match options.open(&self.path) {Ok(file)=>file,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(retirement_pending().lock().map_err(|_|"input retirement lock poisoned")?.get(&self.path).cloned()),Err(e)=>return Err(e.to_string())};
        validate_private_file(&file)?;
        let mut bytes=Vec::new(); Read::by_ref(&mut file).take(MAX_BYTES as u64+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
        if bytes.len()>MAX_BYTES { return Err("input intent too large".into()); }
        let intent:Intent=serde_json::from_slice(&bytes).map_err(|_|"invalid input intent; recovery required")?;
        if intent.version!=1 || intent.database!=self.database.to_string_lossy() || intent.source_owner!=SOURCE_OWNER || intent.session!=self.session
            || intent.request_id.is_empty() || intent.submission_id.is_empty() || intent.created_at_ms<0 {
            return Err("input intent owner mismatch".into());
        }
        let submission:nola_core::agent::AgentSubmission=serde_json::from_str(&intent.payload_json).map_err(|_|"invalid frozen input payload")?;
        match submission {
            nola_core::agent::AgentSubmission::UserInput {submission_id:Some(id),submitted_at_epoch_ms:Some(time),delivery:nola_core::agent::AgentInputDelivery::NextTurn,..}
                if id==intent.submission_id && time==intent.created_at_ms=>{},
            _=>return Err("input intent payload identity mismatch".into()),
        }
        // A previous writer may have published the rename but failed its
        // directory sync. Visibility alone must not authorize sending the
        // frozen request: establish durable recovery before any delivery.
        file.sync_all().map_err(|e|e.to_string())?;
        sync_directory(&self.directory)?;
        Ok(Some(intent))
    }
    fn create(&self,intent:&Intent)->Result<(),String> {
        let bytes=serde_json::to_vec(intent).map_err(|e|e.to_string())?;
        if bytes.len()>MAX_BYTES {return Err("input intent too large".into());}
        // Same-directory temporary write + durable publication. The persistent
        // lock covers inspection, creation, delivery and retirement.
        let temp=self.directory.join(format!("{}.tmp",uuid::Uuid::new_v4()));
        let mut file=private_file(&temp,true)?; file.write_all(&bytes).map_err(|e|e.to_string())?;
        file.sync_all().map_err(|e|e.to_string())?;
        fs::rename(&temp,&self.path).map_err(|e|e.to_string())?;
        sync_directory(&self.directory)
    }
    fn clear(&self)->Result<(),String> {
        match fs::remove_file(&self.path) {Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},Err(e)=>return Err(e.to_string())}
        sync_directory(&self.directory)
    }
}

fn retire_confirmed(journal:&Journal,intent:&Intent,cleanup:impl FnOnce()->Result<(),String>) {
    // Poison before unlink: no failing cleanup may reopen NewSubmit in this
    // process. Canonical admission remains Durable even when retirement fails.
    let mut pending=retirement_pending().lock().unwrap_or_else(|p|p.into_inner());
    pending.insert(journal.path.clone(),intent.clone());
    if cleanup().is_ok() {pending.remove(&journal.path);}
}

fn private_directory(path:&Path)->Result<(),String> {
    let metadata=fs::symlink_metadata(path).map_err(|e|e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {return Err("input journal must be a real directory".into());}
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o777 != 0o700 || metadata.uid() != unsafe {libc::geteuid()} {
            return Err("input journal directory must be privately owned".into());
        }
    }
    #[cfg(windows)] private_windows_path(path)?;
    Ok(())
}
fn private_file(path:&Path,exclusive:bool)->Result<File,String> {
    let mut options=OpenOptions::new(); options.read(true).write(true);
    if exclusive {options.create_new(true);} else {options.create(true);}
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt; options.mode(0o600).custom_flags(libc::O_NOFOLLOW);}
    let file=options.open(path).map_err(|e|e.to_string())?;
    #[cfg(windows)] private_windows_path(path)?;
    validate_private_file(&file)?;
    Ok(file)
}
fn validate_private_file(file:&File)->Result<(),String> {
    let metadata=file.metadata().map_err(|e|e.to_string())?;
    if !metadata.is_file() {return Err("input intent must be a regular file".into());}
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o777 != 0o600 || metadata.uid() != unsafe {libc::geteuid()} || metadata.nlink()!=1 {
            return Err("input intent must be a privately owned, unshared file".into());
        }
    }
    Ok(())
}
#[cfg(windows)]
fn private_windows_path(path:&Path)->Result<(),String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::{SetFileSecurityW,DACL_SECURITY_INFORMATION,PROTECTED_DACL_SECURITY_INFORMATION};
    use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    // Protected DACL: only the file owner receives full access; do not inherit
    // a potentially public custom database-directory ACL onto input text.
    let descriptor:Vec<u16>="D:P(A;;FA;;;OW)\0".encode_utf16().collect();
    let mut security=std::ptr::null_mut();
    let path:Vec<u16>=path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(descriptor.as_ptr(),1,&mut security,std::ptr::null_mut())==0 {return Err(std::io::Error::last_os_error().to_string());}
        let result=SetFileSecurityW(path.as_ptr(),DACL_SECURITY_INFORMATION|PROTECTED_DACL_SECURITY_INFORMATION,security);
        let error=if result==0 {Some(std::io::Error::last_os_error().to_string())} else {None};
        windows_sys::Win32::Foundation::LocalFree(security);
        if let Some(error)=error {return Err(error);}
    }
    Ok(())
}
fn sync_directory(path:&Path)->Result<(),String> {
    let mut options=OpenOptions::new(); options.read(true);
    #[cfg(windows)] {use std::os::windows::fs::OpenOptionsExt; options.custom_flags(0x02000000);}
    options.open(path).and_then(|file|file.sync_all()).map_err(|e|e.to_string())
}

pub(crate) fn execute(session:&str,operation:InputOperation<'_>)->Result<String,String> {
    let database=crate::store::default_db_path()?;
    execute_with_delivery(&database,session,operation,|database,intent| {
        let mut application=crate::application_port::CliApplicationPort::persistent(database)?;
        let input=nola_application::AgentCommand {session_id:intent.session.clone(),command_id:intent.submission_id.clone(),payload_json:intent.payload_json.clone(),created_at_ms:intent.created_at_ms};
        if application.reconcile_input_admission(&intent.request_id,input.clone(),false)? {return Ok(true);}
        application.reconcile_input_admission(&intent.request_id,input,true)
    })
}

fn execute_with_delivery(
    database:&Path,session:&str,operation:InputOperation<'_>,
    mut deliver:impl FnMut(&Path,&Intent)->Result<bool,String>,
)->Result<String,String> {
    let journal=Journal::open(&database,session)?;
    let existing=journal.read()?;
    if matches!(operation,InputOperation::Inspect) {
        return Ok(existing.map(|intent|format!("/input-recovery resume {}",intent.request_id)).unwrap_or_else(||"No pending input intent".into()));
    }
    let intent=match operation {
        InputOperation::NewSubmit(text)=>{
            if let Some(existing)=existing { return Err(format!("Input admission pending; /input-recovery resume {}",existing.request_id)); }
            if text.trim().is_empty() {return Err("empty input".into());}
            let submission_id=format!("turn-steer:{}",uuid::Uuid::new_v4());
            let created_at_ms=chrono::Utc::now().timestamp_millis();
            let mut items=vec![nola_core::agent::AgentItem::user_message(nola_core::agent::ItemId::from_string(submission_id.clone()),text)];
            crate::composer_media::append_items(&mut items,&submission_id,text);
            let submission=nola_core::agent::AgentSubmission::UserInput {
                items,
                turn_context_items:vec![],turn_context_platform:nola_core::context_engine::ContextPlatform::Cli,turn_context_components:vec![],
                submission_id:Some(submission_id.clone()),submitted_at_epoch_ms:Some(created_at_ms),delivery:nola_core::agent::AgentInputDelivery::NextTurn,
            };
            let intent=Intent {version:1,database:journal.database.to_string_lossy().into_owned(),source_owner:SOURCE_OWNER.into(),session:session.into(),
                request_id:format!("input-admission:{}",uuid::Uuid::new_v4()),submission_id,payload_json:serde_json::to_string(&submission).map_err(|e|e.to_string())?,created_at_ms};
            journal.create(&intent)?; intent
        }
        InputOperation::ResumePending(request)=>{
            let intent=existing.ok_or("no pending input intent")?;
            if request!=intent.request_id {return Err("pending input identity mismatch".into());} intent
        }
        InputOperation::Inspect=>unreachable!(),
    };
    // The journal's canonical database path proves file ownership. Transport
    // still uses the original Application locator: its socket/client identity
    // is derived from that exact locator, not from the journal's path spelling.
    if !deliver(database,&intent)? {
        return Err(format!("Input admission pending; /input-recovery resume {}",intent.request_id));
    }
    retire_confirmed(&journal,&intent,||journal.clear());
    Ok(intent.submission_id)
}

pub(crate) fn ensure_no_pending(session:&str)->Result<(),String> {
    let journal=Journal::open(&crate::store::default_db_path()?,session)?;
    if let Some(intent)=journal.read()? {
        return Err(format!("Input admission pending; /input-recovery resume {}",intent.request_id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn journal_owner_normalization_does_not_replace_the_application_transport_locator() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let actual = root.path().join("actual");
        fs::create_dir(&actual).unwrap();
        let alias = root.path().join("address");
        symlink(&actual, &alias).unwrap();
        let database = alias.join("db.sqlite");
        let canonical_database = actual.canonicalize().unwrap().join("db.sqlite");
        assert_ne!(database, canonical_database);
        let mut request_id = None;
        assert!(execute_with_delivery(&database, "session", InputOperation::NewSubmit("follow-up"),
            |transport, intent| {
                assert_eq!(transport, database);
                assert_eq!(Path::new(&intent.database), canonical_database);
                request_id = Some(intent.request_id.clone());
                Ok(false)
            }).is_err());
        execute_with_delivery(&database, "session", InputOperation::ResumePending(&request_id.unwrap()),
            |transport, intent| {
                assert_eq!(transport, database);
                assert_eq!(Path::new(&intent.database), canonical_database);
                Ok(true)
            }).unwrap();
        assert!(Journal::open(&database, "session").unwrap().read().unwrap().is_none());
    }
    #[cfg(unix)]
    #[test]
    fn exposed_intent_lock_or_directory_is_rejected_without_sending_or_chmod() {
        use std::os::unix::fs::PermissionsExt;
        for target in ["intent", "lock", "directory"] {
            let root=tempfile::tempdir().unwrap();
            let database=root.path().join("db.sqlite");
            assert!(execute_with_delivery(&database,"session",InputOperation::NewSubmit("private input"),|_,_|Ok(false)).is_err());
            let journal=Journal::open(&database,"session").unwrap();
            let intent=journal.read().unwrap().unwrap();
            let intent_path=journal.path.clone();
            let original=fs::read(&intent_path).unwrap();
            let path=match target {
                "intent"=>intent_path.clone(),
                "lock"=>intent_path.with_extension("lock"),
                _=>journal.directory.clone(),
            };
            drop(journal);
            let mode=if target=="directory" {0o755} else {0o644};
            fs::set_permissions(&path,fs::Permissions::from_mode(mode)).unwrap();
            assert!(execute_with_delivery(&database,"session",InputOperation::ResumePending(&intent.request_id),
                |_,_|panic!("untrusted journal must not send")).is_err(),"{target}");
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777,mode);
            assert_eq!(fs::read(&intent_path).unwrap(),original);
        }
    }
    #[test]
    fn lost_ack_reopens_frozen_intent_and_never_reinterprets_new_identical_text() {
        let directory=tempfile::tempdir().unwrap(); let database=directory.path().join("db.sqlite");
        let mut original=None;
        assert!(execute_with_delivery(&database,"session",InputOperation::NewSubmit("/literal text"),|_,intent| {
            original=Some(serde_json::to_value(intent).unwrap()); Err("lost ACK after commit".into())
        }).is_err());
        let id=original.as_ref().unwrap()["request_id"].as_str().unwrap().to_owned();
        let status=execute_with_delivery(&database,"session",InputOperation::Inspect,|_,_|panic!("inspect cannot send")).unwrap();
        assert!(status.contains(&id));
        assert!(execute_with_delivery(&database,"session",InputOperation::NewSubmit("/literal text"),|_,_|panic!("pending cannot be replaced")).is_err());
        assert!(execute_with_delivery(&database,"other-session",InputOperation::ResumePending(&id),|_,_|panic!("wrong owner cannot send")).is_err());
        execute_with_delivery(&database,"session",InputOperation::ResumePending(&id),|_,intent| {
            assert_eq!(Some(serde_json::to_value(intent).unwrap()),original); Ok(true)
        }).unwrap();
        assert!(Journal::open(&database,"session").unwrap().read().unwrap().is_none());
    }
    #[test]
    fn malformed_intent_and_pending_receipt_never_mean_absent() {
        let directory=tempfile::tempdir().unwrap(); let database=directory.path().join("db.sqlite");
        assert!(execute_with_delivery(&database,"session",InputOperation::NewSubmit("follow-up"),|_,_|Ok(false)).is_err());
        let journal=Journal::open(&database,"session").unwrap();
        assert!(journal.read().unwrap().is_some());
        let mut file=OpenOptions::new().write(true).truncate(true).open(&journal.path).unwrap();
        file.write_all(b"{bad").unwrap();file.sync_all().unwrap();drop(file);drop(journal);
        assert!(execute_with_delivery(&database,"session",InputOperation::NewSubmit("replacement"),|_,_|panic!("corruption cannot send")).is_err());
    }
    #[test]
    fn unlink_sync_failure_preserves_process_poison_but_not_reverts_durable_ack() {
        let directory=tempfile::tempdir().unwrap();let database=directory.path().join("db.sqlite");
        assert!(execute_with_delivery(&database,"session",InputOperation::NewSubmit("text"),|_,_|Ok(false)).is_err());
        let journal=Journal::open(&database,"session").unwrap();let intent=journal.read().unwrap().unwrap();
        retire_confirmed(&journal,&intent,||{fs::remove_file(&journal.path).unwrap();Err("directory sync failure".into())});
        assert_eq!(journal.read().unwrap().unwrap().request_id,intent.request_id);drop(journal);
        assert!(execute_with_delivery(&database,"session",InputOperation::NewSubmit("text"),|_,_|panic!("poison blocks new intent")).is_err());
        execute_with_delivery(&database,"session",InputOperation::ResumePending(&intent.request_id),|_,saved|{assert_eq!(saved.request_id,intent.request_id);Ok(true)}).unwrap();
        assert!(Journal::open(&database,"session").unwrap().read().unwrap().is_none());
    }
    #[test]
    fn receipt_survives_consumer_ack_and_changed_payload_cannot_enqueue_again() {
        let mut application=crate::application_port::CliApplicationPort::ephemeral().unwrap();
        let payload=nola_core::agent::AgentSubmission::UserInput {
            items:vec![],turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
            submission_id:Some("input".into()),submitted_at_epoch_ms:Some(1),delivery:nola_core::agent::AgentInputDelivery::NextTurn,
        };
        let input=nola_application::AgentCommand {session_id:"session".into(),command_id:"input".into(),payload_json:serde_json::to_string(&payload).unwrap(),created_at_ms:1};
        assert!(!application.reconcile_input_admission("request",input.clone(),false).unwrap());
        assert!(application.reconcile_input_admission("request",input.clone(),true).unwrap());
        let lease:Option<nola_application::AgentCommandLease>=application.execute(nola_application::ApplicationCommand::ClaimNextAgentCommand {
            session_id:"session".into(),lease_owner:"consumer".into(),now_ms:2,lease_until_ms:100,
        }).unwrap();
        assert!(lease.is_some());
        let _:bool=application.execute(nola_application::ApplicationCommand::AcknowledgeAgentCommand {
            session_id:"session".into(),command_id:"input".into(),lease_owner:"consumer".into(),
        }).unwrap();
        assert!(application.reconcile_input_admission("request",input.clone(),false).unwrap());
        let mut changed=input;changed.created_at_ms=3;
        assert!(application.reconcile_input_admission("request",changed,true).is_err());
        let lease:Option<nola_application::AgentCommandLease>=application.execute(nola_application::ApplicationCommand::ClaimNextAgentCommand {
            session_id:"session".into(),lease_owner:"consumer".into(),now_ms:3,lease_until_ms:100,
        }).unwrap();assert!(lease.is_none());
    }
}
