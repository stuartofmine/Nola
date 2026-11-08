// SPDX-License-Identifier: Apache-2.0
// Acceptance reads only actual Application Service public journal commands.
use nola_domain::assistant_journal::{ASSISTANT_RANGE_MAX_BYTES, ASSISTANT_RANGE_MIN_BYTES};
use serde_json::{json, Value};
use std::collections::HashSet;

pub struct JournalService {
    pub client: nola_service_client::ApplicationServiceClient,
    child: Option<std::process::Child>,
}

impl JournalService {
    pub fn open(database: &std::path::Path) -> Self {
        let socket = nola_service_client::socket_path_for_database(database);
        let client = nola_service_client::ApplicationServiceClient::with_socket(database, &socket)
            .with_request_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let mut service = Self {
            client,
            child: None,
        };
        if service.client.health().is_err() {
            service.child = Some(
                std::process::Command::new(env!("CARGO_BIN_EXE_nola"))
                    .arg("--nola-application-service")
                    .arg("--database")
                    .arg(database)
                    .arg("--socket")
                    .arg(socket)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::inherit())
                    .spawn()
                    .expect("start exact fixture Application Service"),
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while service.client.health().is_err() {
                assert!(service
                    .child
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .unwrap()
                    .is_none());
                assert!(
                    std::time::Instant::now() < deadline,
                    "fixture Service startup failed"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        service
    }

    pub fn query(&self, command: serde_json::Value) -> serde_json::Value {
        self.client
            .execute(command)
            .expect("actual Service journal query")
    }
}

impl Drop for JournalService {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn number(value: &Value) -> u64 {
    let text = value.as_str().expect("lossless decimal journal counter");
    let number = text.parse::<u64>().unwrap();
    assert_eq!(number.to_string(), text);
    number
}

pub fn owner_head(service: &JournalService, conversation: &str, run: &str, message: &str) -> Value {
    let mut after: Option<String> = None;
    loop {
        let heads = service.query(json!({"type":"local_run_assistant_journal_heads",
            "conversation_id":conversation,"run_id":run,"after_message_id":after,"limit":32}));
        let page = heads.as_array().expect("public owner heads");
        assert!(page.len() <= 32);
        let mut previous = after.clone();
        for head in page {
            assert_eq!(head["identity"]["conversation_id"], conversation);
            assert_eq!(head["identity"]["run_id"], run);
            let id = head["identity"]["assistant_message_id"].as_str().unwrap();
            assert!(previous.as_deref().is_none_or(|value| value < id));
            previous = Some(id.into());
        }
        if let Some(head) = page
            .iter()
            .find(|head| head["identity"]["assistant_message_id"] == message)
        {
            return head.clone();
        }
        assert_eq!(page.len(), 32, "canonical assistant has no published head");
        after = previous;
    }
}

struct SnapshotLease<'a> {
    service: &'a JournalService,
    snapshot: Value,
    released: bool,
}
impl Drop for SnapshotLease<'_> {
    fn drop(&mut self) {
        if !self.released {
            // Never mask a failed assertion with a second panic during release.
            let _: Result<Value, String> = self.service.client.execute(json!({
                "type":"release_assistant_journal_snapshot","snapshot":self.snapshot
            }));
        }
    }
}

fn order(part: &Value) -> (u8, u64, &str) {
    let section = match part["order_key"]["section"].as_str().unwrap() {
        "transcript" => 0,
        "tasks" => 1,
        "skills" => 2,
        "model_attempts" => 3,
        other => panic!("unknown journal section {other}"),
    };
    (
        section,
        number(&part["order_key"]["position"]),
        part["part_id"].as_str().unwrap(),
    )
}

// Directory cursors are opaque even when they contain a Part ID.
#[derive(Default)]
pub struct DirectoryProgress {
    seen: HashSet<String>,
}
impl DirectoryProgress {
    pub fn advance(&mut self, next: Option<&str>) -> Option<String> {
        next.map(|next| {
            assert!(
                !next.is_empty() && self.seen.insert(next.to_owned()),
                "journal directory cursor cycle"
            );
            next.to_owned()
        })
    }
}

pub fn read_public_text(service: &JournalService, through: &Value) -> String {
    let snapshot = service.query(json!({"type":"open_assistant_journal_snapshot","at":through}));
    let mut lease = SnapshotLease {
        service,
        snapshot,
        released: false,
    };
    assert_eq!(lease.snapshot["at"], *through);
    let mut after: Option<String> = None;
    let mut progress = DirectoryProgress::default();
    let mut parts = Vec::new();
    let mut ids = HashSet::new();
    loop {
        let page = service.query(
            json!({"type":"read_assistant_journal_parts","snapshot":lease.snapshot,
            "after_part_id":after,"max_records":32,"max_encoded_bytes":65536}),
        );
        assert_eq!(page["at"], *through);
        assert!(
            serde_json::to_vec(&page).unwrap().len() <= 65536,
            "parts page exceeded wire budget"
        );
        let entries = page["parts"].as_array().unwrap();
        assert!(entries.len() <= 32);
        for part in entries {
            assert_eq!(part["identity"], through["identity"]);
            assert!(
                ids.insert(part["part_id"].as_str().unwrap().to_owned()),
                "duplicate journal Part"
            );
            if let Some(previous) = parts.last() {
                assert!(
                    order(previous) < order(part),
                    "cold directory display order regressed"
                );
            }
            parts.push(part.clone());
        }
        assert!(page["next_part_id"].is_null() || page["next_part_id"].is_string());
        after = progress.advance(page["next_part_id"].as_str());
        if after.is_none() {
            break;
        }
        // An empty/retired page may still have a continuation.
    }
    parts.sort_by(|a, b| order(a).cmp(&order(b)));
    let mut answer = String::new();
    for part in parts {
        let metadata = &part["metadata"];
        let review = &metadata["data"]["review"];
        if metadata["kind"] != "text"
            || metadata["data"]["visibility"] != "public"
            || !(review.is_null() || review == "approved")
        {
            continue;
        }
        let body = part["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|body| body["field"] == "markdown")
            .expect("public markdown reference");
        let total = number(&body["utf8_bytes"]);
        let mut offset = 0;
        // Also prove EOF against the source for an empty body.
        loop {
            const LIMIT: usize = ASSISTANT_RANGE_MAX_BYTES;
            assert!((ASSISTANT_RANGE_MIN_BYTES..=ASSISTANT_RANGE_MAX_BYTES).contains(&LIMIT));
            let range = service.query(json!({"type":"read_assistant_journal_range","snapshot":lease.snapshot,
                "part_id":part["part_id"],"content":body,"offset_bytes":offset.to_string(),"limit_bytes":LIMIT}));
            let text = range["text"].as_str().unwrap();
            let next = number(&range["next_offset_bytes"]);
            assert_eq!(number(&range["offset_bytes"]), offset);
            assert_eq!(number(&range["total_utf8_bytes"]), total);
            assert!(text.len() <= LIMIT && next >= offset && next <= total);
            assert_eq!(next - offset, text.len() as u64);
            assert_eq!(range["end_of_content"], next == total);
            assert!(
                next > offset || offset == total,
                "range made no UTF-8 progress"
            );
            answer.push_str(text);
            offset = next;
            if offset == total {
                break;
            }
        }
    }
    service.query(json!({"type":"release_assistant_journal_snapshot","snapshot":lease.snapshot}));
    lease.released = true;
    answer
}

// Exercise empty pages against the actual source's published directory, not a
// fabricated snapshot response. All current entries postdate the initial cut.
pub fn assert_empty_genesis_pages(service: &JournalService, through: &Value) -> usize {
    let identity = serde_json::from_value(through["identity"].clone()).unwrap();
    let genesis =
        nola_domain::assistant_journal::AssistantJournalCursor::initial(identity).unwrap();
    let at = serde_json::to_value(genesis).unwrap();
    let snapshot = service.query(json!({"type":"open_assistant_journal_snapshot","at":at}));
    let mut lease = SnapshotLease {
        service,
        snapshot,
        released: false,
    };
    assert_eq!(lease.snapshot["at"], at);
    let mut after: Option<String> = None;
    let mut progress = DirectoryProgress::default();
    let mut continuations = 0;
    loop {
        let page = service.query(
            json!({"type":"read_assistant_journal_parts","snapshot":lease.snapshot,
            "after_part_id":after,"max_records":1,"max_encoded_bytes":65536}),
        );
        assert_eq!(page["at"], at);
        assert!(
            page["parts"].as_array().unwrap().is_empty(),
            "postdated Part leaked into genesis"
        );
        assert!(page["next_part_id"].is_null() || page["next_part_id"].is_string());
        after = progress.advance(page["next_part_id"].as_str());
        if after.is_none() {
            break;
        }
        continuations += 1;
    }
    service.query(json!({"type":"release_assistant_journal_snapshot","snapshot":lease.snapshot}));
    lease.released = true;
    continuations
}

pub fn public_text(
    database: &std::path::Path,
    conversation: &str,
    run: &str,
    message: &str,
) -> String {
    let service = JournalService::open(database);
    let through = owner_head(&service, conversation, run, message);
    read_public_text(&service, &through)
}

#[test]
fn directory_progress_accepts_opaque_empty_page_hops_and_rejects_cycles() {
    let mut progress = DirectoryProgress::default();
    assert_eq!(
        progress.advance(Some("z-retired")),
        Some("z-retired".into())
    );
    assert_eq!(progress.advance(Some("a-empty")), Some("a-empty".into()));
    assert_eq!(progress.advance(None), None);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || progress.advance(Some("z-retired"))
    ))
    .is_err());
}
