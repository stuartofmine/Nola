// SPDX-License-Identifier: Apache-2.0
//! Production SDK upload, publication and original readers against restarted Services.
#![cfg(unix)]
use nola_application::{ApplicationCommand, CreateConversation};
use nola_protocol::{
    content_upload::{
        AppendContentChunk, BeginContentUpload, ContentUploadOwner, ContentUploadProgress,
        ContentUploadRequest,
    },
    uploaded_transcript::{TranscriptPartUpload, UploadedTranscriptManifest},
    CommandReceiptOutcome, ConversationRecallCommand, ConversationRecallHit, ConversationTextRange,
    ConversationTranscriptImportSource, ExportBodyField, ExportCommand, ExportPage, ExportRange,
    ImportConversationTranscript, TranscriptCommand, TranscriptMessageInput, TranscriptPartInput,
};
use nola_service_client::ApplicationServiceClient;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(database: &Path, socket: &Path) -> Service {
    let mut service = Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .arg("--database")
            .arg(database)
            .arg("--socket")
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let client = ApplicationServiceClient::with_socket(database, socket);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return service;
        }
        assert!(service.0.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(20));
    }
    panic!("publication Service startup timed out");
}

// Transparent framing observer. SDK and Service still encode/decode every real
// envelope; this does not synthesize responses or call Application in-process.
struct Frames {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    request: Arc<AtomicUsize>,
    response: Arc<AtomicUsize>,
    count: Arc<AtomicUsize>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Frames {
    fn start(path: &Path, target: &Path) -> Self {
        let listener = UnixListener::bind(path).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let request = Arc::new(AtomicUsize::new(0));
        let response = Arc::new(AtomicUsize::new(0));
        let count = Arc::new(AtomicUsize::new(0));
        let (done, req, res, n) = (
            stop.clone(),
            request.clone(),
            response.clone(),
            count.clone(),
        );
        let target = target.to_owned();
        let worker = thread::spawn(move || {
            for stream in listener.incoming() {
                if done.load(Ordering::Relaxed) {
                    break;
                }
                let mut stream = stream.unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                let mut frame = Vec::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_until(b'\n', &mut frame)
                    .unwrap();
                if frame.is_empty() {
                    continue;
                }
                req.fetch_max(frame.len(), Ordering::Relaxed);
                let mut upstream = UnixStream::connect(&target).unwrap();
                upstream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                upstream.write_all(&frame).unwrap();
                frame.clear();
                BufReader::new(upstream)
                    .read_until(b'\n', &mut frame)
                    .unwrap();
                res.fetch_max(frame.len(), Ordering::Relaxed);
                n.fetch_add(1, Ordering::Relaxed);
                stream.write_all(&frame).unwrap();
            }
        });
        Self {
            path: path.to_owned(),
            stop,
            request,
            response,
            count,
            worker: Some(worker),
        }
    }
}
impl Drop for Frames {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = UnixStream::connect(&self.path);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

struct Memory {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<(u64, u64, usize)>>,
}
impl Memory {
    fn start(service_pid: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            let (mut service_peak, mut client_peak, mut samples) = (0, 0, 0);
            let client_pid = std::process::id();
            while !done.load(Ordering::Relaxed) {
                let output = Command::new("ps")
                    .args([
                        "-o",
                        "pid=,rss=",
                        "-p",
                        &format!("{service_pid},{client_pid}"),
                    ])
                    .output()
                    .unwrap();
                assert!(output.status.success(), "RSS query failed");
                for line in String::from_utf8(output.stdout).unwrap().lines() {
                    let values: Vec<u64> = line
                        .split_whitespace()
                        .map(|value| value.parse().unwrap())
                        .collect();
                    if values[0] == u64::from(service_pid) {
                        service_peak = service_peak.max(values[1] * 1024);
                    }
                    if values[0] == u64::from(client_pid) {
                        client_peak = client_peak.max(values[1] * 1024);
                    }
                }
                samples += 1;
                thread::sleep(Duration::from_millis(40));
            }
            (service_peak, client_peak, samples)
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }
    fn finish(mut self) -> (u64, u64, usize) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap()
    }
}
impl Drop for Memory {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn counts(database: &Path) -> (i64, i64, i64, i64, i64) {
    let db =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    db.query_row(
        "SELECT (SELECT COUNT(*) FROM messages),(SELECT COUNT(*) FROM message_parts),
        (SELECT revision FROM conversations WHERE id='c'),(SELECT COUNT(*) FROM sync_outbox),
        (SELECT COUNT(*) FROM transcript_import_publications)",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )
    .unwrap()
}

#[test]
fn sdk_upload_publish_replay_and_original_readers_survive_real_service_restarts() {
    let started = Instant::now();
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("published.db");
    let socket = root.path().join("service.sock");
    let proxy = root.path().join("frames.sock");
    let mut service = start(&database, &socket);
    let frames = Frames::start(&proxy, &socket);
    let client = ApplicationServiceClient::with_socket(&database, &proxy);
    let body = "界\\\"\n".repeat(2_499_997) + "TAIL-SENTINL";
    assert_eq!(body.chars().count(), 10_000_000);
    let mut memory = Memory::start(service.0.id());
    let mut samples = Vec::new();
    let _: Value = client
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "c".into(),
                title: "Uploaded original".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    let owner = ContentUploadOwner {
        conversation_id: "c".into(),
        import_id: "import".into(),
        part_id: "p".into(),
    };
    let request = ContentUploadRequest {
        upload_id: "upload".into(),
        owner: owner.clone(),
    };
    let progress: ContentUploadProgress = client
        .execute(ApplicationCommand::BeginContentUpload {
            input: BeginContentUpload {
                upload_id: "upload".into(),
                owner: owner.clone(),
                expected_utf8_bytes: body.len() as u64,
            },
        })
        .unwrap();
    assert_eq!(progress.committed_utf8_bytes, 0);
    let mut uploaded = 0;
    let mut chunks = 0;
    for (index, bytes) in body.as_bytes().chunks(30_000).enumerate() {
        let text = std::str::from_utf8(bytes).unwrap().to_owned();
        let progress: ContentUploadProgress = client
            .execute_with_request_id(
                format!("chunk-{index}"),
                ApplicationCommand::AppendContentChunk {
                    input: AppendContentChunk {
                        upload_id: "upload".into(),
                        owner: owner.clone(),
                        offset_bytes: uploaded,
                        text,
                    },
                },
            )
            .unwrap();
        uploaded += bytes.len() as u64;
        chunks += 1;
        assert_eq!(progress.committed_utf8_bytes, uploaded);
        if index == 249 {
            samples.push(memory.finish());
            drop(service);
            service = start(&database, &socket);
            memory = Memory::start(service.0.id());
            let progress: ContentUploadProgress = client
                .execute(ApplicationCommand::ContentUploadProgress {
                    input: request.clone(),
                })
                .unwrap();
            assert_eq!(progress.committed_utf8_bytes, uploaded);
        }
    }
    let _: ContentUploadProgress = client
        .execute(ApplicationCommand::SealContentUpload {
            input: request.clone(),
        })
        .unwrap();
    assert_eq!(
        counts(&database),
        (0, 0, 1, 1, 0),
        "sealed staging must not publish messages"
    );
    samples.push(memory.finish());
    drop(service);
    service = start(&database, &socket);
    memory = Memory::start(service.0.id());
    let input = UploadedTranscriptManifest {
        expected_conversation_revision: 1,
        transcript: ImportConversationTranscript {
            import_id: "import".into(),
            source: ConversationTranscriptImportSource::BackupRestore,
            conversation_id: "c".into(),
            expected_message_count: 0,
            imported_at_ms: 2,
            messages: vec![TranscriptMessageInput {
                id: "m".into(),
                run_id: None,
                role: "user".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(2),
                parts: vec![TranscriptPartInput {
                    id: "p".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: String::new(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 2,
                }],
            }],
        },
        uploads: vec![TranscriptPartUpload {
            upload_id: "upload".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            expected_utf8_bytes: uploaded,
        }],
    };
    let command = TranscriptCommand::PublishUploadedTranscript {
        input: input.clone(),
    };
    let receipt = client
        .submit_with_request_id("publish", command.clone())
        .unwrap();
    let CommandReceiptOutcome::Committed {
        result: ref published,
        ..
    } = receipt.outcome
    else {
        panic!("publication failed: {receipt:?}")
    };
    let committed = counts(&database);
    assert_eq!(committed, (1, 1, 2, chunks + 3, 1));
    samples.push(memory.finish());
    drop(service);
    service = start(&database, &socket);
    memory = Memory::start(service.0.id());
    assert_eq!(client.command_receipt("publish").unwrap().unwrap(), receipt);
    assert_eq!(
        client
            .submit_with_request_id("publish", command.clone())
            .unwrap(),
        receipt
    );
    let replay: Value = client
        .execute_with_request_id("publish-new-request", command)
        .unwrap();
    assert_eq!(&replay, published);
    assert_eq!(counts(&database), committed);
    let mut changed = input.clone();
    changed.transcript.imported_at_ms = 3;
    assert!(client
        .execute_with_request_id::<Value, _>(
            "changed-import",
            TranscriptCommand::PublishUploadedTranscript {
                input: changed.clone()
            }
        )
        .is_err());
    assert!(client
        .execute_with_request_id::<Value, _>(
            "publish",
            TranscriptCommand::PublishUploadedTranscript { input: changed }
        )
        .is_err());
    assert_eq!(counts(&database), committed);
    let hits: Vec<ConversationRecallHit> = client
        .execute(ConversationRecallCommand::SearchConversationText {
            conversation_id: "c".into(),
            query: "TAIL-SENTINL".into(),
            limit: 5,
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].part_id, "p");
    assert!(hits[0].snippet.contains("TAIL-SENTINL"));
    let window: Value = client
        .execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
            conversation_id: "c".into(),
            before_ordinal: None,
            limit: 10,
            content_byte_limit: 65_536,
        })
        .unwrap();
    assert!(window.to_string().len() < 100_000);
    assert_eq!(
        window["projection"]["messages"].as_array().unwrap().len(),
        1
    );
    let page: Option<ExportPage> = client
        .execute(ExportCommand::LoadConversationExportPage {
            conversation_id: "c".into(),
            after_ordinal: None,
            expected_revision: Some(2),
        })
        .unwrap();
    assert_eq!(
        page.unwrap().messages[0].parts[0].content_text_utf8_bytes,
        uploaded
    );
    let before_read = client.command_receipt("publish").unwrap().unwrap();
    let mut offset = 0;
    let mut range_reads = 0;
    while offset < body.len() {
        let range: ConversationTextRange = client
            .execute(ConversationRecallCommand::ReadConversationTextRange {
                conversation_id: "c".into(),
                message_id: "m".into(),
                part_id: "p".into(),
                offset_bytes: offset as u64,
                limit_bytes: 32768,
            })
            .unwrap();
        assert_eq!(range.text, body[offset..offset + range.text.len()]);
        assert_eq!(range.total_utf8_bytes, uploaded);
        assert!(range.next_offset_bytes > offset as u64);
        let export: ExportRange = client
            .execute(ExportCommand::ReadConversationExportRange {
                conversation_id: "c".into(),
                message_id: "m".into(),
                part_id: "p".into(),
                field: ExportBodyField::ContentText,
                expected_revision: 2,
                offset_bytes: offset as u64,
                limit_bytes: 32768,
            })
            .unwrap();
        assert_eq!(export.text, range.text);
        assert_eq!(export.next_offset_bytes, range.next_offset_bytes);
        offset = range.next_offset_bytes as usize;
        range_reads += 1;
    }
    assert_eq!(counts(&database), committed);
    assert_eq!(
        client.command_receipt("publish").unwrap().unwrap(),
        before_read
    );
    samples.push(memory.finish());
    let request_max = frames.request.load(Ordering::Relaxed);
    let response_max = frames.response.load(Ordering::Relaxed);
    assert!(
        request_max < 256 * 1024 && response_max < 256 * 1024,
        "actual wire frames must remain bounded"
    );
    let service_peak = samples.iter().map(|sample| sample.0).max().unwrap();
    let client_peak = samples.iter().map(|sample| sample.1).max().unwrap();
    assert!(
        service_peak < 128 * 1024 * 1024,
        "Service RSS exceeds bounded publication budget"
    );
    eprintln!(
        "uploaded_publication_ipc {}",
        json!({"characters":10000000,"original_bytes":uploaded,"chunks":chunks,
        "service_restarts":3,"range_reads":range_reads,"export_reads":range_reads,"wire_exchanges":frames.count.load(Ordering::Relaxed),
        "maximum_request_frame_bytes":request_max,"maximum_response_frame_bytes":response_max,
        "sampled_service_peak_rss_bytes":service_peak,"sampled_client_peak_rss_bytes":client_peak,
        "rss_samples":samples.iter().map(|sample|sample.2).sum::<usize>(),"elapsed_ms":started.elapsed().as_millis(),
        "same_request_receipt_replay":true,"different_request_import_replay":true,"changed_replay_rejected":true,"single_part":true})
    );
    drop(frames);
    drop(service);
}
