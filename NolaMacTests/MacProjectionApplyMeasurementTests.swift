// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class MacProjectionApplyMeasurementTests: XCTestCase {
    private func ticket(generation: UInt64 = 7) -> MacProjectionApplyMeasurement.Ticket {
        .init(id: "observation", session: MacProjectionApplyMeasurement.sessionDigest("abc"),
            stage: .windowRefresh, generation: generation, startedNS: 100)
    }

    func testSessionHashMatchesRustRawIDConventionAndCaptureIsOnlyProxy() throws {
        XCTAssertEqual(MacProjectionApplyMeasurement.sessionDigest("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        let sample = try XCTUnwrap(MacProjectionApplyMeasurement.sample(ticket: ticket(), outcome: .applied,
            cursor: .init(generation: 7, eventSequence: 42, projectionRevision: 12), nowNS: 145, selected: false))
        XCTAssertEqual(sample.stage, "window_refresh_proxy")
        XCTAssertEqual(sample.elapsedNS, 45)
        XCTAssertEqual(sample.monotonicNS, 145)
        XCTAssertEqual(sample.eventSequence, 42)
        XCTAssertEqual(sample.projectionRevision, 12)
        XCTAssertFalse(sample.selected, "background window apply is not selected publication")
        XCTAssertNotEqual(sample.session, "abc")
    }

    func testStaleApplyOrUnchangedCannotBorrowNewGenerationCursor() throws {
        for outcome in [MacProjectionApplyMeasurement.Outcome.applied, .unchanged] {
            let sample = try XCTUnwrap(MacProjectionApplyMeasurement.sample(ticket: ticket(), outcome: outcome,
                cursor: .init(generation: 8, eventSequence: 99, projectionRevision: 88), nowNS: 145, selected: false))
            XCTAssertEqual(sample.outcome, "discarded")
            XCTAssertNil(sample.eventSequence)
            XCTAssertNil(sample.projectionRevision)
            XCTAssertEqual(sample.generation, 7)
        }
    }

    func testFailedCancelledAndMissingCursorDoNotInventAppliedEvidence() throws {
        for outcome in [MacProjectionApplyMeasurement.Outcome.failed, .cancelled, .discarded] {
            let sample = try XCTUnwrap(MacProjectionApplyMeasurement.sample(ticket: ticket(), outcome: outcome,
                cursor: .init(generation: 7, eventSequence: 42, projectionRevision: 12), nowNS: 145, selected: false))
            XCTAssertNil(sample.eventSequence)
            XCTAssertNil(sample.projectionRevision)
            XCTAssertEqual(sample.outcome, outcome.rawValue)
        }
        XCTAssertNil(MacProjectionApplyMeasurement.sample(ticket: ticket(), outcome: .applied,
            cursor: nil, nowNS: 99, selected: false), "reverse monotonic time must not become zero latency")
        let missing = try XCTUnwrap(MacProjectionApplyMeasurement.sample(ticket: ticket(), outcome: .unchanged,
            cursor: nil, nowNS: 145, selected: false))
        XCTAssertNil(missing.eventSequence)
    }
}
