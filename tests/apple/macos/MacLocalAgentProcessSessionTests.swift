import Darwin
import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacLocalAgentProcessSessionTests: XCTestCase {
    private enum InjectedWriteFailure: Error { case unavailable }

    private struct DescriptorIdentity {
        let number: Int32
        let device: dev_t
        let inode: ino_t
        init(_ number: Int32) {
            self.number = number
            var info = stat()
            precondition(fstat(number, &info) == 0)
            device = info.st_dev
            inode = info.st_ino
        }
    }

    private struct Fixture {
        let process: Process
        let input: Pipe
        let output: Pipe
        let error: Pipe
        let session: MacLocalAgentProcessSession
        let descriptors: [DescriptorIdentity]

        init(
            registry: MacOwnedChildProcessRegistry,
            executable: String = "/bin/cat",
            writer: @escaping @Sendable (FileHandle, Data) throws -> Void = {
                try $0.write(contentsOf: $1)
            }
        ) {
            process = Process()
            process.executableURL = URL(fileURLWithPath: executable)
            input = Pipe()
            output = Pipe()
            error = Pipe()
            descriptors = [input, output, error].flatMap {
                [DescriptorIdentity($0.fileHandleForReading.fileDescriptor), DescriptorIdentity($0.fileHandleForWriting.fileDescriptor)]
            }
            session = MacLocalAgentProcessSession(
                process: process, inputPipe: input, outputPipe: output, errorPipe: error,
                registry: registry, writeInitial: writer
            )
        }
    }

    private func ticket(_ generation: UInt64 = 1) -> MacRunSubmissionTicket {
        .init(identity: .init(conversationID: "isolated-process", submissionID: "run-\(generation)",
                              generationID: generation))
    }

    private func assertClosed(_ fixture: Fixture, file: StaticString = #filePath, line: UInt = #line) {
        for descriptor in fixture.descriptors {
            var current = stat()
            errno = 0
            if fstat(descriptor.number, &current) == -1 {
                XCTAssertEqual(errno, EBADF, file: file, line: line)
            } else {
                // Foundation may close a child's unused pipe end at launch.
                // Other test-host work can immediately recycle its number.
                // Check the original pipe identity, not process-wide FD reuse.
                XCTAssertFalse(current.st_dev == descriptor.device && current.st_ino == descriptor.inode,
                    "owned pipe \(descriptor.number) is still open", file: file, line: line)
            }
        }
    }

    func testInitialWriteFailureAfterLaunchDrainsOwnedChildWithoutInventingPreDispatchRejection() async throws {
        let registry = MacOwnedChildProcessRegistry()
        let submission = ticket()
        var fixture: Fixture? = Fixture(registry: registry, writer: { _, _ in
            throw InjectedWriteFailure.unavailable
        })
        weak var child: Process?
        child = fixture?.process
        try fixture!.session.launch(ticket: submission)
        XCTAssertTrue(fixture!.process.isRunning)
        XCTAssertEqual(submission.phase, .dispatching)
        XCTAssertThrowsError(try fixture!.session.writeInitialRequest(Data("request\n".utf8)))
        XCTAssertEqual(submission.phase, .dispatching, "a started child may have observed dispatch")

        // Mirrors the runtime's post-launch catch boundary, not a simulated
        // pre-launch rejection. Only this isolated fixture child is stopped.
        await fixture!.session.shutdownAndDrain()
        XCTAssertFalse(fixture!.process.isRunning)
        XCTAssertTrue(fixture!.session.isReleased)
        assertClosed(fixture!)
        XCTAssertEqual(submission.phase, .dispatching)
        fixture = nil
        for _ in 0..<100 where child != nil {
            try await Task.sleep(for: .milliseconds(10))
        }
        withExtendedLifetime(registry) {
            XCTAssertNil(child, "the isolated ownership registry must release the exited child")
        }
    }

    func testMissingExecutableIsDefinitePreTransportRejectionAndClosesPipes() async {
        let fixture = Fixture(registry: MacOwnedChildProcessRegistry(),
            executable: "/nola-test-missing-executable-\(UUID().uuidString)")
        let submission = ticket()
        XCTAssertThrowsError(try fixture.session.launch(ticket: submission))
        XCTAssertEqual(submission.phase, .rejectedBeforeDispatch)
        XCTAssertFalse(fixture.process.isRunning)
        await fixture.session.shutdownAndDrain()
        XCTAssertTrue(fixture.session.isReleased)
        assertClosed(fixture)
    }

    func testAbandonedTicketNeverLaunchesChild() async {
        let fixture = Fixture(registry: MacOwnedChildProcessRegistry())
        let submission = ticket()
        XCTAssertTrue(submission.cancelPreparation())
        XCTAssertThrowsError(try fixture.session.launch(ticket: submission))
        XCTAssertEqual(submission.phase, .abandoned)
        XCTAssertFalse(fixture.process.isRunning)
        await fixture.session.shutdownAndDrain()
        XCTAssertTrue(fixture.session.isReleased)
        assertClosed(fixture)
    }

    func testLateOldSessionCleanupDoesNotCloseOrStopReplacementChild() async throws {
        let registry = MacOwnedChildProcessRegistry()
        let old = Fixture(registry: registry)
        try old.session.launch(ticket: ticket())
        await old.session.shutdownAndDrain()
        XCTAssertTrue(old.session.isReleased)

        // New pipes may reuse the old descriptor numbers. Repeated cleanup
        // must respect session ownership, not close recycled raw descriptors.
        let replacement = Fixture(registry: registry)
        let nextTicket = ticket(2)
        try replacement.session.launch(ticket: nextTicket)
        _ = old.session.releaseResources()
        await old.session.shutdownAndDrain()
        XCTAssertTrue(replacement.process.isRunning)
        XCTAssertFalse(replacement.session.isReleased)
        XCTAssertEqual(nextTicket.phase, .dispatching)
        XCTAssertNoThrow(try replacement.session.writeInitialRequest(Data("still owned\n".utf8)))
        await replacement.session.shutdownAndDrain()
        XCTAssertFalse(replacement.process.isRunning)
        assertClosed(replacement)
    }

    func testNormalExitIsObservedWithoutStoppingUnrelatedOwnedChild() async throws {
        let registry = MacOwnedChildProcessRegistry()
        let unrelated = Fixture(registry: registry)
        try unrelated.session.launch(ticket: ticket())
        let finished = Fixture(registry: registry, executable: "/usr/bin/true")
        try finished.session.launch(ticket: ticket(2))
        let exit = await finished.session.waitForExit()
        XCTAssertEqual(exit.status, 0)
        XCTAssertFalse(exit.signaled)
        _ = finished.session.releaseResources()
        XCTAssertTrue(finished.session.isReleased)
        assertClosed(finished)
        await finished.session.shutdownAndDrain()
        XCTAssertTrue(unrelated.process.isRunning)
        await unrelated.session.shutdownAndDrain()
        XCTAssertFalse(unrelated.process.isRunning)
    }
}
