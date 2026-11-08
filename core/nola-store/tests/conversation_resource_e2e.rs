use base64::{engine::general_purpose::STANDARD, Engine as _};
use nola_store::NolaStore;
use nola_store_port::{
    conversation_profile::CreateConversationInput,
    conversation_resource::{
        ConversationResourceRepository, PutConversationPdfResource, MAX_PDF_RESOURCE_BYTES,
    },
};
use std::sync::{Arc, Barrier};

fn pdf() -> Vec<u8> {
    b"%PDF-1.4\n1 0 obj <<>> endobj\n%%EOF\n".to_vec()
}
fn input(conversation_id: &str, resource_id: String, bytes: &[u8]) -> PutConversationPdfResource {
    PutConversationPdfResource {
        conversation_id: conversation_id.into(),
        resource_id,
        display_name: "报告.pdf".into(),
        mime_type: "application/pdf".into(),
        data_base64: STANDARD.encode(bytes),
        created_at_ms: 1,
    }
}
fn create(store: &mut NolaStore, id: &str) {
    store
        .create_conversation(&CreateConversationInput {
            id: id.into(),
            title: id.into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
}

#[test]
fn pdf_resource_is_immutable_conversation_scoped_and_cascades_on_delete() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("resources.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    create(&mut store, "one");
    create(&mut store, "two");
    let input = input("one", uuid::Uuid::new_v4().to_string(), &pdf());
    let descriptor = store
        .put_conversation_pdf_resource(&input)
        .unwrap()
        .unwrap();
    assert_eq!(descriptor.byte_count, pdf().len() as u64);
    assert_eq!(
        store
            .put_conversation_pdf_resource(&input)
            .unwrap()
            .unwrap(),
        descriptor
    );
    assert!(store
        .read_conversation_resource("two", &input.resource_id)
        .unwrap()
        .is_none());
    assert!(!store
        .delete_conversation_resource("two", &input.resource_id)
        .unwrap());
    let read = store
        .read_conversation_resource("one", &input.resource_id)
        .unwrap()
        .unwrap();
    assert_eq!(STANDARD.decode(read.data_base64).unwrap(), pdf());
    let listed = store.list_conversation_resources("one").unwrap().unwrap();
    assert_eq!(listed, vec![descriptor.clone()]);
    assert_eq!(
        store.list_conversation_resources("two").unwrap().unwrap(),
        Vec::new()
    );
    let mut changed = input.clone();
    changed.data_base64 = STANDARD.encode(b"%PDF-1.4\nother\n%%EOF\n");
    assert!(store.put_conversation_pdf_resource(&changed).is_err());
    assert!(store.delete_conversation("one").unwrap());
    assert!(store
        .read_conversation_resource("one", &input.resource_id)
        .unwrap()
        .is_none());
}

#[test]
fn pdf_resource_rejects_spoofing_oversize_paths_and_missing_owner() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(root.path().join("resources.sqlite")).unwrap();
    create(&mut store, "one");
    let id = uuid::Uuid::new_v4().to_string();
    let invalid = [
        b"not a PDF\n%%EOF\n".to_vec(),
        b"%PDF-1.4\nmissing trailer".to_vec(),
        vec![b'x'; MAX_PDF_RESOURCE_BYTES + 1],
    ];
    for bytes in invalid {
        assert!(store
            .put_conversation_pdf_resource(&input("one", id.clone(), &bytes))
            .is_err());
    }
    let mut metadata = input("one", id.clone(), &pdf());
    metadata.display_name = "../report.pdf".into();
    assert!(store.put_conversation_pdf_resource(&metadata).is_err());
    metadata.display_name = "report.pdf".into();
    metadata.mime_type = "text/html".into();
    assert!(store.put_conversation_pdf_resource(&metadata).is_err());
    assert!(store
        .put_conversation_pdf_resource(&input("missing", id, &pdf()))
        .unwrap()
        .is_none());
}

#[test]
fn concurrent_pdf_uploads_observe_one_transactional_conversation_quota() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("resources.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    create(&mut store, "one");
    drop(store);
    let barrier = Arc::new(Barrier::new(12));
    let workers = (0..12)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let database = database.clone();
            std::thread::spawn(move || {
                let mut store = NolaStore::open(database).unwrap();
                let input = input("one", uuid::Uuid::new_v4().to_string(), &pdf());
                barrier.wait();
                store.put_conversation_pdf_resource(&input).is_ok()
            })
        })
        .collect::<Vec<_>>();
    let successes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .filter(|succeeded| *succeeded)
        .count();
    assert_eq!(successes, 10);
    let connection = rusqlite::Connection::open(database).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM conversation_pdf_resources WHERE conversation_id='one'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 10);
}

#[test]
fn incompatible_schema_is_rejected_without_rewriting_existing_pdf_or_conversation() {
    for version in [nola_store::STORE_SCHEMA_VERSION-1, nola_store::STORE_SCHEMA_VERSION+1] {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("incompatible.sqlite");
        let mut store = NolaStore::open(&database).unwrap();
        create(&mut store,"existing");
        let resource = input("existing",uuid::Uuid::new_v4().to_string(),&pdf());
        store.put_conversation_pdf_resource(&resource).unwrap().unwrap();
        drop(store);
        let sql = rusqlite::Connection::open(&database).unwrap();
        sql.pragma_update(None,"user_version",version).unwrap();
        sql.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        drop(sql);
        let before = std::fs::read(&database).unwrap();
        assert!(matches!(NolaStore::open(&database),Err(nola_store::StoreError::IncompatibleSchema)));
        assert_eq!(std::fs::read(&database).unwrap(),before,"failed open rewrote schema {version}");
        let sql = rusqlite::Connection::open(&database).unwrap();
        let body:Vec<u8> = sql.query_row("SELECT body FROM conversation_pdf_resources WHERE resource_id=?1",
            [&resource.resource_id],|row|row.get(0)).unwrap();
        assert_eq!(body,pdf());
    }
}
