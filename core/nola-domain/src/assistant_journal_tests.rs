// SPDX-License-Identifier: Apache-2.0
use super::*;

fn golden() -> serde_json::Value {
    serde_json::from_str(include_str!("../tests/fixtures/assistant_journal_v1.json")).unwrap()
}
fn identity() -> AssistantJournalIdentity {
    serde_json::from_value(golden()["identity"].clone()).unwrap()
}
fn digest(byte: &str) -> AssistantDigest {
    AssistantDigest::parse_hex(&byte.repeat(32)).unwrap()
}
fn record(sequence: u64, previous: AssistantDigest) -> AssistantJournalRecordHeaderV1 {
    AssistantJournalRecordHeaderV1::seal(
        identity(),
        sequence,
        previous,
        "operation-1".into(),
        7,
        digest("11"),
    )
    .unwrap()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn raw_range_budget_is_distinct_from_chunk_and_frame_budgets() {
    assert_eq!(ASSISTANT_RANGE_MIN_BYTES, 4);
    assert_eq!(ASSISTANT_RANGE_MAX_BYTES, 32768);
    assert!(ASSISTANT_RANGE_MIN_BYTES <= ASSISTANT_RANGE_MAX_BYTES);
    assert!(ASSISTANT_RANGE_MAX_BYTES < ASSISTANT_CHUNK_MAX_BYTES);
    assert!(ASSISTANT_CHUNK_MAX_BYTES < ASSISTANT_FRAME_MAX_BYTES);
}

#[test]
fn canonical_encoding_matches_independent_cross_language_golden_vectors() {
    // Golden hashes generated independently from the specified binary layout,
    // not by serializing these Rust structs or regenerating expected values.
    let fixture = golden();
    let cursor = AssistantJournalCursor::initial(identity()).unwrap();
    assert_eq!(cursor.record_digest.to_hex(), fixture["genesis_digest"]);
    let chunk = AssistantTextChunkV1::new(fixture["chunk_text"].as_str().unwrap().into()).unwrap();
    assert_eq!(chunk.byte_length(), 11);
    assert_eq!(
        hex(&chunk.canonical_bytes().unwrap()),
        fixture["chunk_canonical_hex"]
    );
    assert_eq!(chunk.digest().to_hex(), fixture["chunk_digest"]);
    let record = record(1, cursor.record_digest);
    assert_eq!(
        hex(&record.canonical_bytes().unwrap()),
        fixture["record_canonical_hex"]
    );
    assert_eq!(record.record_digest.to_hex(), fixture["record_digest"]);
    record.verify().unwrap();
    chunk.verify().unwrap();
}

#[test]
fn json_presentation_does_not_define_record_identity() {
    let record = record(
        1,
        AssistantJournalCursor::initial(identity())
            .unwrap()
            .record_digest,
    );
    let compact = serde_json::to_string(&record).unwrap();
    let pretty = serde_json::to_string_pretty(&record).unwrap();
    assert_ne!(compact.as_bytes(), pretty.as_bytes());
    for json in [compact, pretty] {
        let decoded: AssistantJournalRecordHeaderV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(
            decoded.canonical_bytes().unwrap(),
            record.canonical_bytes().unwrap()
        );
        decoded.verify().unwrap();
    }
}

#[test]
fn apply_replay_conflict_gap_and_historical_replay_are_distinct() {
    let initial = AssistantJournalCursor::initial(identity()).unwrap();
    let first = record(1, initial.record_digest);
    assert_eq!(initial.admit(&first), Ok(AssistantRecordAdmission::Apply));
    assert_eq!(
        initial.sequence, 0,
        "admission must not advance durable state"
    );
    let head = AssistantJournalCursor {
        identity: identity(),
        sequence: 1,
        record_digest: first.record_digest,
    };
    assert_eq!(head.admit(&first), Ok(AssistantRecordAdmission::Replay));
    let conflicting = AssistantJournalRecordHeaderV1::seal(
        identity(),
        1,
        initial.record_digest,
        "different-operation".into(),
        7,
        digest("11"),
    )
    .unwrap();
    assert_eq!(
        head.admit(&conflicting),
        Err(AssistantJournalError::ConflictingReplay)
    );
    assert_eq!(
        head.admit(&record(3, first.record_digest)),
        Err(AssistantJournalError::SequenceGap {
            expected: 2,
            received: 3
        })
    );
    assert_eq!(
        head.admit(&record(2, digest("22"))),
        Err(AssistantJournalError::PreviousDigestMismatch)
    );
    let second = record(2, first.record_digest);
    assert_eq!(head.admit(&second), Ok(AssistantRecordAdmission::Apply));
    let later = AssistantJournalCursor {
        sequence: 2,
        record_digest: second.record_digest,
        ..head
    };
    assert_eq!(
        later.admit(&first),
        Err(AssistantJournalError::HistoricalVerificationRequired { sequence: 1 })
    );
}

#[test]
fn all_owner_fields_and_source_epoch_are_bound() {
    let original = AssistantJournalCursor::initial(identity()).unwrap();
    for index in 0..4 {
        let mut changed = identity();
        match index {
            0 => changed.source_store_epoch.push('2'),
            1 => changed.conversation_id.push('2'),
            2 => changed.run_id.push('2'),
            _ => changed.assistant_message_id.push('2'),
        }
        let cursor = AssistantJournalCursor::initial(changed.clone()).unwrap();
        assert_ne!(original.record_digest, cursor.record_digest);
        let record = AssistantJournalRecordHeaderV1::seal(
            changed,
            1,
            cursor.record_digest,
            "operation-1".into(),
            7,
            digest("11"),
        )
        .unwrap();
        assert_eq!(
            original.admit(&record),
            Err(AssistantJournalError::IdentityMismatch)
        );
    }
}

#[test]
fn digest_tampering_and_unknown_versions_fail_closed() {
    let initial = AssistantJournalCursor::initial(identity()).unwrap();
    let mut changed = record(1, initial.record_digest);
    changed.source_commit_revision += 1;
    assert_eq!(
        initial.admit(&changed),
        Err(AssistantJournalError::RecordDigestMismatch)
    );
    changed.protocol_version = 2;
    assert_eq!(
        changed.verify(),
        Err(AssistantJournalError::UnsupportedVersion)
    );
    let mut bad_genesis = initial.clone();
    bad_genesis.record_digest = digest("00");
    assert_eq!(
        bad_genesis.admit(&record(1, initial.record_digest)),
        Err(AssistantJournalError::PreviousDigestMismatch)
    );
}

#[test]
fn chunks_are_bounded_in_utf8_bytes_not_characters_and_never_normalized() {
    let max = AssistantTextChunkV1::new("🙂".repeat(ASSISTANT_CHUNK_MAX_BYTES / 4)).unwrap();
    assert_eq!(max.byte_length(), ASSISTANT_CHUNK_MAX_BYTES);
    assert_eq!(
        AssistantTextChunkV1::new(format!("{}a", max.text())),
        Err(AssistantJournalError::ChunkTooLarge)
    );
    assert_eq!(
        AssistantTextChunkV1::new(String::new()),
        Err(AssistantJournalError::EmptyChunk)
    );
    assert_ne!(
        AssistantTextChunkV1::new("é".into()).unwrap().digest(),
        AssistantTextChunkV1::new("e\u{301}".into())
            .unwrap()
            .digest()
    );
    let mut wire = serde_json::to_value(max).unwrap();
    wire["text"] = "mutated".into();
    let decoded: AssistantTextChunkV1 = serde_json::from_value(wire).unwrap();
    assert_eq!(
        decoded.verify(),
        Err(AssistantJournalError::RecordDigestMismatch)
    );
}

#[test]
fn malformed_digest_identity_and_frame_budgets_are_rejected() {
    for value in [
        "00".repeat(31),
        "AA".repeat(32),
        "gg".repeat(32),
        format!(" {}", "00".repeat(32)),
    ] {
        assert!(AssistantDigest::parse_hex(&value).is_err());
        assert!(serde_json::from_value::<AssistantDigest>(serde_json::json!(value)).is_err());
    }
    for value in [
        " ".into(),
        "bad\nowner".into(),
        "a".repeat(ASSISTANT_ID_MAX_BYTES + 1),
    ] {
        let mut owner = identity();
        owner.run_id = value;
        assert!(AssistantJournalCursor::initial(owner).is_err());
    }
    assert!(
        validate_assistant_page_budget(ASSISTANT_FRAME_MAX_BYTES, ASSISTANT_PAGE_MAX_RECORDS)
            .is_ok()
    );
    assert_eq!(
        validate_assistant_page_budget(ASSISTANT_FRAME_MAX_BYTES + 1, 1),
        Err(AssistantJournalError::BudgetExceeded)
    );
    assert_eq!(
        validate_assistant_page_budget(1, ASSISTANT_PAGE_MAX_RECORDS + 1),
        Err(AssistantJournalError::BudgetExceeded)
    );
    assert!(validate_assistant_record_budget(
        ASSISTANT_FRAME_MAX_BYTES,
        ASSISTANT_RECORD_MAX_OPERATIONS
    )
    .is_ok());
    assert!(validate_assistant_record_budget(1, 0).is_err());
    assert!(validate_assistant_record_budget(1, ASSISTANT_RECORD_MAX_OPERATIONS + 1).is_err());
    assert!(validate_assistant_record_budget(ASSISTANT_FRAME_MAX_BYTES + 1, 1).is_err());
}

#[test]
fn byte_length_prefixes_prevent_ambiguous_identity_concatenation() {
    let mut a = identity();
    a.conversation_id = "ab".into();
    a.run_id = "c".into();
    let mut b = identity();
    b.conversation_id = "a".into();
    b.run_id = "bc".into();
    assert_ne!(
        AssistantJournalCursor::initial(a).unwrap().record_digest,
        AssistantJournalCursor::initial(b).unwrap().record_digest
    );
}

#[test]
fn unknown_wire_fields_and_invalid_utf8_are_not_accepted_as_chunks() {
    let mut wire = serde_json::to_value(AssistantTextChunkV1::new("ok".into()).unwrap()).unwrap();
    wire["trace_sequence"] = 1.into();
    assert!(serde_json::from_value::<AssistantTextChunkV1>(wire).is_err());
    assert!(serde_json::from_slice::<AssistantTextChunkV1>(b"{\"text\":\"\xff\"}").is_err());
}

#[test]
fn maximum_u64_roundtrips_losslessly_as_decimal_strings() {
    let genesis = AssistantJournalCursor::initial(identity()).unwrap();
    let record = AssistantJournalRecordHeaderV1::seal(
        identity(),
        u64::MAX,
        genesis.record_digest,
        "operation-1".into(),
        u64::MAX,
        digest("11"),
    )
    .unwrap();
    let fixture = golden();
    assert_eq!(
        record.record_digest.to_hex(),
        fixture["u64_max_record_digest"]
    );
    let wire = serde_json::to_value(&record).unwrap();
    assert_eq!(wire["sequence"], fixture["u64_max_wire"]);
    assert_eq!(wire["source_commit_revision"], fixture["u64_max_wire"]);
    let decoded: AssistantJournalRecordHeaderV1 = serde_json::from_value(wire).unwrap();
    assert_eq!(decoded, record);
    decoded.verify().unwrap();
    let cursor = AssistantJournalCursor {
        identity: identity(),
        sequence: u64::MAX,
        record_digest: record.record_digest,
    };
    let wire = serde_json::to_value(&cursor).unwrap();
    assert_eq!(wire["sequence"], fixture["u64_max_wire"]);
    assert_eq!(
        serde_json::from_value::<AssistantJournalCursor>(wire).unwrap(),
        cursor
    );
    assert_eq!(cursor.admit(&record), Ok(AssistantRecordAdmission::Replay));
}

#[test]
fn numeric_json_signed_fractional_padded_and_overflow_cursors_are_rejected() {
    let initial = AssistantJournalCursor::initial(identity()).unwrap();
    let record = record(1, initial.record_digest);
    for bad in [
        serde_json::json!(1),
        serde_json::json!(null),
        serde_json::json!(true),
        serde_json::json!("+1"),
        serde_json::json!("-1"),
        serde_json::json!("01"),
        serde_json::json!("1.0"),
        serde_json::json!("1e3"),
        serde_json::json!(" 1"),
        serde_json::json!(""),
        serde_json::json!("18446744073709551616"),
    ] {
        let mut cursor = serde_json::to_value(&initial).unwrap();
        cursor["sequence"] = bad.clone();
        assert!(
            serde_json::from_value::<AssistantJournalCursor>(cursor).is_err(),
            "{bad}"
        );
        for field in ["sequence", "source_commit_revision"] {
            let mut wire = serde_json::to_value(&record).unwrap();
            wire[field] = bad.clone();
            assert!(
                serde_json::from_value::<AssistantJournalRecordHeaderV1>(wire).is_err(),
                "{field}: {bad}"
            );
        }
    }
    assert_eq!(serde_json::to_value(initial).unwrap()["sequence"], "0");
}
