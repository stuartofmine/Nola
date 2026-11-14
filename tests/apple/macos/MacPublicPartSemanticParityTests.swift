// SPDX-License-Identifier: Apache-2.0
import XCTest
import SwiftUI
import AppKit
import Combine
@testable import Nola

@MainActor
final class MacPublicPartSemanticParityTests: XCTestCase {
    func testReadOnlyCardCanReconcileOriginalSubmissionWithoutDuplicateDispatch() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let original = MacA2UIRenderFixture(nodeCount: 4).snapshot
        let nodes = original.nodes.mapValues { node in
            AppleUINode(instance_id: node.instance_id, component_id: node.component_id,
                component: node.component, props: node.props, slots: node.slots, bindings: [:], actions: [:],
                validation_errors: node.validation_errors)
        }
        let closed = AppleUISnapshot(identity: original.identity, revision: 2, catalog_id: original.catalog_id,
            catalog_digest: original.catalog_digest, phase: .readOnly, root_instance_id: original.root_instance_id,
            nodes: nodes, summary: original.summary)
        let pending = AppleUISubmission<MacCanonicalJSONValue>(interaction: .init(identity: original.identity,
            expected_surface_revision: 1, edits: [], action: nil), submission_id: "original-request", submitted_at_ms: 123)
        var submissions: [String] = []
        let host = NSHostingView(rootView: AppleUIReferencedSurfaceView(reference: .init(identity: original.identity, revision: 1),
            summary: "Form", readChanges: { _, _ in .snapshot(closed) },
            preview: { _ in throw NSError(domain: "Fixture", code: 1) }, readRevision: { _ in nil },
            pendingSubmission: { _ in pending }, submit: { input in
                submissions.append(input.submission_id)
                try? await Task.sleep(for: .milliseconds(120))
                return .unknown("Waiting for the original receipt")
            }))
        host.frame = NSRect(x: 0, y: 0, width: 500, height: 400)
        let window = NSWindow(contentRect: host.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await Task.sleep(for: .milliseconds(100))
        host.layoutSubtreeIfNeeded()
        func buttons(_ object: AnyObject) -> [AnyObject] {
            let own = (object.accessibilityRole?() ?? nil)?.rawValue == "AXButton" ? [object] : []
            return own + ((object.accessibilityChildren?() ?? nil) ?? []).flatMap { buttons($0 as AnyObject) }
        }
        let actions = buttons(host)
        XCTAssertEqual(actions.count, 1, "Closed content still needs its pending receipt recovery action")
        let action = try XCTUnwrap(actions.first)
        _ = action.accessibilityPerformPress?()
        _ = action.accessibilityPerformPress?()
        try await Task.sleep(for: .milliseconds(250))
        XCTAssertEqual(submissions, ["original-request"])
    }

    func testReadOnlyTransitionShowsCommittedValuesAndRetainsUnsentDraftForReopening() async throws {
        let fixture = MacA2UIRenderFixture(nodeCount: 4)
        let original = fixture.snapshot
        let host = NSHostingView(rootView: MacA2UIRenderProbe(fixture: fixture))
        host.frame = NSRect(x: 0, y: 0, width: 500, height: 300)
        let window = NSWindow(contentRect: host.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        host.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(60))
        func input(_ view: NSView) -> NSTextField? {
            if let field = view as? NSTextField, field.placeholderString == "IME probe" { return field }
            return view.subviews.lazy.compactMap { input($0) }.first
        }
        let field = try XCTUnwrap(input(host))
        field.selectText(nil)
        let editor = try XCTUnwrap(field.currentEditor() as? NSTextView)
        editor.insertText("本地未提交", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        window.makeFirstResponder(nil)
        try await Task.sleep(for: .milliseconds(80))
        func snapshot(_ phase: AppleUISurfacePhase, revision: UInt64) -> AppleUISnapshot<MacCanonicalJSONValue> {
            .init(identity: original.identity, revision: revision, catalog_id: original.catalog_id,
                catalog_digest: original.catalog_digest, phase: phase, root_instance_id: original.root_instance_id,
                nodes: original.nodes, summary: original.summary)
        }
        fixture.snapshot = snapshot(.readOnly, revision: 2)
        try await Task.sleep(for: .milliseconds(80))
        XCTAssertEqual(try XCTUnwrap(input(host)).stringValue, "", "Read-only content must show committed values")
        fixture.snapshot = snapshot(.ready, revision: 3)
        try await Task.sleep(for: .milliseconds(80))
        XCTAssertEqual(try XCTUnwrap(input(host)).stringValue, "本地未提交", "Changing phase must not discard an unsent draft")
    }

    func testPendingAndRemovedFormsHideRetainedControls() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = MacA2UIRenderFixture(nodeCount: 4).snapshot
        for phase in [AppleUISurfacePhase.pending, .deleted] {
            let snapshot = AppleUISnapshot(identity: fixture.identity, revision: fixture.revision,
                catalog_id: fixture.catalog_id, catalog_digest: fixture.catalog_digest, phase: phase,
                root_instance_id: fixture.root_instance_id, nodes: fixture.nodes, summary: fixture.summary)
            let host = NSHostingView(rootView: AppleUISurfaceView(snapshot: snapshot,
                submit: { _ in XCTFail("Inactive form must not submit"); return .rejected("Fixture") }, accepted: { _ in }))
            host.frame = NSRect(x: 0, y: 0, width: 360, height: 300)
            let window = NSWindow(contentRect: host.bounds, styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
            window.makeKeyAndOrderFront(nil)
            defer { window.orderOut(nil); window.contentView = nil; window.close() }
            host.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(60))
            func roles(_ object: AnyObject) -> [String] {
                let role = (object.accessibilityRole?() ?? nil)?.rawValue ?? ""
                return [role] + ((object.accessibilityChildren?() ?? nil) ?? []).flatMap { roles($0 as AnyObject) }
            }
            let rendered = roles(host)
            XCTAssertFalse(rendered.contains("AXTextField"), "\(phase) must not show a retained input")
            XCTAssertTrue(rendered.contains("AXStaticText"), "\(phase) needs an explicit visible status")
        }
    }

    func testReadySurfaceDoesNotAppendDuplicateHistoryDisclosure() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = MacA2UIRenderFixture(nodeCount: 4)
        let snapshot = fixture.snapshot
        var reads = 0
        let reference = AppleUISurfaceReference(identity: snapshot.identity, revision: snapshot.revision)
        let host = NSHostingView(rootView: AppleUIReferencedSurfaceView<MacCanonicalJSONValue>(
            reference: reference, summary: "fixture", readChanges: { _, _ in reads += 1; return .snapshot(snapshot) },
            preview: { _ in throw NSError(domain: "Fixture", code: 1) },
            readRevision: { _ in XCTFail("Live content must not request a duplicate historical copy"); return nil },
            pendingSubmission: { _ in nil }, submit: { _ in .rejected("Fixture only") }))
        host.frame = NSRect(x: 0, y: 0, width: 500, height: 400)
        let window = NSWindow(contentRect: host.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        let deadline = ContinuousClock.now + .seconds(2)
        while reads == 0 && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        host.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(60))
        XCTAssertGreaterThan(reads, 0)
        func hasDisclosure(_ object: AnyObject) -> Bool {
            if (object.accessibilityRole?() ?? nil)?.rawValue == "AXDisclosureTriangle" { return true }
            return ((object.accessibilityChildren?() ?? nil) ?? []).contains { hasDisclosure($0 as AnyObject) }
        }
        XCTAssertFalse(hasDisclosure(host), "A ready card must appear once without a version disclosure")
    }

    func testEmptyDateAcceptanceMatchesVisibleConstrainedSelection() {
        let today = Date(timeIntervalSince1970: 1_000)
        let future = Date(timeIntervalSince1970: 2_000)
        let past = Date(timeIntervalSince1970: 500)
        XCTAssertEqual(AppleUIDateInput.emptySelectionDate(now: today, minimum: future, maximum: nil), future)
        XCTAssertEqual(AppleUIDateInput.emptySelectionDate(now: today, minimum: nil, maximum: past), past)
        XCTAssertEqual(AppleUIDateInput.emptySelectionDate(now: today, minimum: past, maximum: future), today)
        XCTAssertEqual(AppleUIDateInput.emptySelectionDate(now: today, minimum: nil, maximum: nil), today)
    }

    func testExplicitDraftRevalidationRebasesWithoutLosingInput() throws {
        let path = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "form_interaction_v0_9_1", withExtension: "json"))
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: path)) as? [String: Any])
        let snapshot = try JSONDecoder().decode(AppleUISnapshot<MacCanonicalJSONValue>.self,
            from: JSONSerialization.data(withJSONObject: XCTUnwrap(fixture["edited_snapshot"])))
        let node = try XCTUnwrap(snapshot.nodes.values.first { !$0.bindings.isEmpty })
        let binding = try XCTUnwrap(node.bindings.keys.first)
        let value = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: Data(#""retained input""#.utf8))
        let state = AppleUIEditingState<MacCanonicalJSONValue>()
        let key = AppleUIEditingState<MacCanonicalJSONValue>.key(node.instance_id, binding)
        state.drafts = [key: value]
        state.draftRevision = snapshot.revision - 1
        state.failure = "stale revision"
        state.revalidate(against: snapshot)
        XCTAssertEqual(state.draftRevision, snapshot.revision)
        XCTAssertEqual(state.drafts[key], value)
        XCTAssertEqual(state.draftGeneration, 1)
        XCTAssertNil(state.failure)

        state.drafts["removed field"] = value
        state.draftRevision = snapshot.revision - 1
        state.revalidate(against: snapshot)
        XCTAssertEqual(state.draftRevision, snapshot.revision - 1)
        XCTAssertEqual(state.draftGeneration, 1)
        XCTAssertEqual(state.drafts.count, 2)
        XCTAssertNotNil(state.failure)

        state.drafts = [key: value]
        state.pending = true
        state.revalidate(against: snapshot)
        XCTAssertEqual(state.draftRevision, snapshot.revision - 1)
        XCTAssertEqual(state.draftGeneration, 1)
    }

    func testDraftPreviewRejectsStaleGenerationRevisionAndForeignOwner() throws {
        let path = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "form_interaction_v0_9_1", withExtension: "json"))
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: path)) as? [String: Any])
        let snapshot = try JSONDecoder().decode(AppleUISnapshot<MacCanonicalJSONValue>.self,
            from: JSONSerialization.data(withJSONObject: XCTUnwrap(fixture["initial_snapshot"])))
        let preview = AppleUIPreviewResult(identity: snapshot.identity, base_revision: snapshot.revision,
            draft_generation: 7, snapshot: snapshot)
        XCTAssertTrue(preview.matches(identity: snapshot.identity, revision: snapshot.revision, generation: 7))
        XCTAssertFalse(preview.matches(identity: snapshot.identity, revision: snapshot.revision, generation: 8))
        XCTAssertFalse(preview.matches(identity: snapshot.identity, revision: snapshot.revision + 1, generation: 7))
        let other = AppleUISurfaceIdentity(owner: snapshot.identity.owner, surface_id: "other", generation: snapshot.identity.generation)
        XCTAssertFalse(preview.matches(identity: other, revision: snapshot.revision, generation: 7))
        let input = AppleUIPreviewInput<MacCanonicalJSONValue>(identity: snapshot.identity,
            expected_surface_revision: snapshot.revision, edits: [], draft_generation: 7)
        let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(AppleUIPreviewCommand(input: input))) as? [String: Any])
        XCTAssertEqual(encoded["type"] as? String, "preview_ui_surface")
        let body = try XCTUnwrap(encoded["input"] as? [String: Any])
        XCTAssertNil(body["action"])
        XCTAssertNil(body["submission_id"])
    }

    func testNumberInputPreservesStringAndRejectsTrailingGarbageAtCommit() {
        let formatter = AppleUINumberFormatter()
        for raw in ["", "-12", "42", "0"] {
            var object: AnyObject?
            var error: NSString?
            XCTAssertTrue(formatter.getObjectValue(&object, for: raw, errorDescription: &error), raw)
            XCTAssertEqual(object as? String, raw)
        }
        for raw in ["12abc", "--1", "hello"] {
            var object: AnyObject?
            var error: NSString?
            XCTAssertFalse(formatter.getObjectValue(&object, for: raw, errorDescription: &error), raw)
        }
        for composing in ["-", "中文", "にほんご", "한국어"] {
            XCTAssertTrue(formatter.isPartialStringValid(composing, newEditingString: nil, errorDescription: nil))
        }
    }

    func testNativeIconsHaveRealSymbolsAndSVGGeometry() throws {
        XCTAssertEqual(AppleUIIconSymbols.names.count, 59)
        for (name, symbol) in AppleUIIconSymbols.names { XCTAssertNotNil(NSImage(systemSymbolName: symbol, accessibilityDescription: name), name) }
        let square = try AppleUISVGPath.parse("M2 2h20v20H2z")
        XCTAssertEqual(square.boundingRect, CGRect(x: 2, y: 2, width: 20, height: 20))
        let arc = try AppleUISVGPath.parse("M2 12 A10 10 0 0122 12 A10 10 0 012 12Z")
        XCTAssertEqual(arc.boundingRect.minX, 2, accuracy: 0.001)
        XCTAssertEqual(arc.boundingRect.maxX, 22, accuracy: 0.001)
        XCTAssertEqual(arc.boundingRect.height, 20, accuracy: 0.001)
        XCTAssertFalse(try AppleUISVGPath.parse("M1e0 2 C2 3 4 5 6 7 s2 3 4 5 Q1 2 3 4 t2 3 L4 5z").isEmpty)
        for invalid in ["", "L1 2", "M0", "MNaN 2", "M0 0 A1 1 0 2 0 2 2", "M0 0X2 2", "M0 0L1e999 2", "M1e308 0l1e308 0"] {
            XCTAssertThrowsError(try AppleUISVGPath.parse(invalid), invalid)
        }
    }

    func testSharedCoreGoldenSurfaceInteractionAndDelta() throws {
        let path = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "form_interaction_v0_9_1", withExtension: "json"))
        let data = try Data(contentsOf: path)
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(fixture["fixture_version"] as? Int, 1)
        func decode<T: Decodable>(_ key: String, as: T.Type) throws -> T {
            try JSONDecoder().decode(T.self, from: JSONSerialization.data(withJSONObject: XCTUnwrap(fixture[key])))
        }
        let identity = try decode("identity", as: AppleUISurfaceIdentity.self)
        let initial = try decode("initial_snapshot", as: AppleUISnapshot<MacCanonicalJSONValue>.self)
        let edited = try decode("edited_snapshot", as: AppleUISnapshot<MacCanonicalJSONValue>.self)
        let delta = try decode("changes", as: AppleUIChangeSet<MacCanonicalJSONValue>.self)
        let interaction = try decode("interaction", as: AppleUIInteraction<MacCanonicalJSONValue>.self)
        let history = try decode("historical_snapshot", as: AppleUISnapshot<MacCanonicalJSONValue>.self)
        XCTAssertEqual(initial.identity, identity)
        XCTAssertEqual(interaction.identity, identity)
        XCTAssertEqual(interaction.expected_surface_revision, initial.revision)
        XCTAssertFalse(interaction.edits.isEmpty)
        XCTAssertNotNil(interaction.action)
        XCTAssertEqual(try AppleUISurfaceUpdate.changes(delta).applying(to: initial, identity: identity), edited)
        XCTAssertEqual(history.identity, identity)
        XCTAssertEqual(history.phase, .readOnly)
        XCTAssertTrue(history.nodes.values.allSatisfy { $0.actions.isEmpty && $0.bindings.isEmpty })
        XCTAssertEqual(initial.identity.accessibilityIdentifier, edited.identity.accessibilityIdentifier)
    }

    func testDraftPreviewEnablesInitiallyInvalidButtonOnlyAfterCompositionCompletes() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let path = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "form_interaction_v0_9_1", withExtension: "json"))
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: path)) as? [String: Any])
        let valid = try JSONDecoder().decode(AppleUISnapshot<MacCanonicalJSONValue>.self,
            from: JSONSerialization.data(withJSONObject: XCTUnwrap(fixture["initial_snapshot"])))
        let buttonNode = try XCTUnwrap(valid.nodes.values.first { $0.component == "Button" })
        var nodes = valid.nodes
        nodes[buttonNode.instance_id] = .init(instance_id: buttonNode.instance_id,
            component_id: buttonNode.component_id, component: buttonNode.component, props: buttonNode.props,
            slots: buttonNode.slots, bindings: buttonNode.bindings, actions: buttonNode.actions, validation_errors: ["Required"])
        let invalid = AppleUISnapshot(identity: valid.identity, revision: valid.revision, catalog_id: valid.catalog_id,
            catalog_digest: valid.catalog_digest, phase: valid.phase, root_instance_id: valid.root_instance_id,
            nodes: nodes, summary: valid.summary)
        var previews = 0
        var submissions = 0
        let host = NSHostingView(rootView: AppleUISurfaceView(snapshot: invalid, preview: { input in
            previews += 1
            XCTAssertFalse(input.edits.isEmpty)
            return AppleUIPreviewResult(identity: input.identity, base_revision: input.expected_surface_revision,
                draft_generation: input.draft_generation, snapshot: valid)
        }, submit: { _ in submissions += 1; return .rejected("Fixture only") }, accepted: { _ in }))
        host.frame = NSRect(x: 0, y: 0, width: 500, height: 500)
        let window = NSWindow(contentRect: host.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        host.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(60))
        func button() throws -> AnyObject {
            var queue: [AnyObject] = [host]
            var seen = Set<ObjectIdentifier>()
            while !queue.isEmpty, seen.count < 4_000 {
                let item = queue.removeFirst()
                guard seen.insert(ObjectIdentifier(item)).inserted else { continue }
                if item.accessibilityIdentifier?() == "a2ui.action." + buttonNode.component_id { return item }
                let children: [Any] = (item.accessibilityChildren?() ?? nil) ?? []
                queue.append(contentsOf: children.map { $0 as AnyObject })
                if let view = item as? NSView { queue.append(contentsOf: view.subviews) }
            }
            throw NSError(domain: "A2UIAXFixture", code: 2)
        }
        func input(_ view: NSView) -> NSTextField? {
            if let field = view as? NSTextField, field.placeholderString == "Name" { return field }
            return view.subviews.lazy.compactMap { input($0) }.first
        }
        XCTAssertEqual(try button().isAccessibilityEnabled?(), false)
        let field = try XCTUnwrap(input(host))
        window.makeFirstResponder(field)
        let editor = try XCTUnwrap(field.currentEditor() as? NSTextView)
        editor.setMarkedText("中文", selectedRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(previews, 0, "IME composition must remain native and must not be validated prematurely")
        XCTAssertEqual(try button().isAccessibilityEnabled?(), false)
        editor.insertText("中文", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        editor.unmarkText()
        let deadline = ContinuousClock.now + .seconds(3)
        while previews == 0 && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(30)) }
        try await Task.sleep(for: .milliseconds(80))
        XCTAssertGreaterThan(previews, 0)
        XCTAssertTrue(field.currentEditor() === editor, "Preview must preserve native input identity/focus")
        XCTAssertEqual(try button().isAccessibilityEnabled?(), true)
        _ = try button().accessibilityPerformPress?()
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertEqual(submissions, 1)
    }

    func testGeneratedButtonLabelKeepsNativeAccessibilityActionEnabled() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let path = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "form_interaction_v0_9_1", withExtension: "json"))
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: path)) as? [String: Any])
        let snapshot = try JSONDecoder().decode(AppleUISnapshot<MacCanonicalJSONValue>.self,
            from: JSONSerialization.data(withJSONObject: XCTUnwrap(fixture["initial_snapshot"])))
        let buttonNode = try XCTUnwrap(snapshot.nodes.values.first { $0.component == "Button" })
        var dispatches = 0
        let host = NSHostingView(rootView: AppleUINodeView(id: buttonNode.instance_id, snapshot: snapshot,
            value: { _, _, fallback in fallback }, edit: { _, _, _ in }, dispatch: { _ in dispatches += 1 }, present: { _ in }))
        host.frame = NSRect(x: 0, y: 0, width: 320, height: 80)
        let window = NSWindow(contentRect: host.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        host.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(50))
        func nativeButton() throws -> AnyObject {
            var queue: [AnyObject] = [host]
            var seen = Set<ObjectIdentifier>()
            while !queue.isEmpty, seen.count < 4_000 {
                let candidate = queue.removeFirst()
                guard seen.insert(ObjectIdentifier(candidate)).inserted else { continue }
                if candidate.accessibilityIdentifier?() == "a2ui.action." + buttonNode.component_id { return candidate }
                let children: [Any] = (candidate.accessibilityChildren?() ?? nil) ?? []
                queue.append(contentsOf: children.map { $0 as AnyObject })
                if let view = candidate as? NSView { queue.append(contentsOf: view.subviews) }
            }
            throw NSError(domain: "A2UIAXFixture", code: 1)
        }
        let button = try nativeButton()
        XCTAssertEqual(button.isAccessibilityEnabled?(), true)
        XCTAssertEqual(button.accessibilityPerformPress?(), true)
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertEqual(dispatches, 1)
        var invalidNodes = snapshot.nodes
        invalidNodes[buttonNode.instance_id] = AppleUINode(instance_id: buttonNode.instance_id,
            component_id: buttonNode.component_id, component: buttonNode.component, props: buttonNode.props,
            slots: buttonNode.slots, bindings: buttonNode.bindings, actions: buttonNode.actions,
            validation_errors: ["Required field is invalid"])
        let invalid = AppleUISnapshot(identity: snapshot.identity, revision: snapshot.revision + 1,
            catalog_id: snapshot.catalog_id, catalog_digest: snapshot.catalog_digest, phase: snapshot.phase,
            root_instance_id: snapshot.root_instance_id, nodes: invalidNodes, summary: snapshot.summary)
        host.rootView = AppleUINodeView(id: buttonNode.instance_id, snapshot: invalid,
            value: { _, _, fallback in fallback }, edit: { _, _, _ in }, dispatch: { _ in dispatches += 1 }, present: { _ in })
        host.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(50))
        let disabled = try nativeButton()
        XCTAssertEqual(disabled.isAccessibilityEnabled?(), false)
        _ = disabled.accessibilityPerformPress?()
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertEqual(dispatches, 1, "Core validation errors must prohibit the native action")
    }

    func testNativeRowConstrictsLongTextAndInputsToNarrowParent() async throws {
        let frames = MacA2UILayoutFrames()
        let root = AppleUIAxisLayout(horizontal: true, justify: "spaceBetween", align: "start") {
            Text(String(repeating: "长中文和 long English text ", count: 8))
                .fixedSize(horizontal: false, vertical: true)
                .background(GeometryReader { proxy in Color.clear.onAppear { frames.values["text"] = proxy.frame(in: .named("layout-probe")) } })
            TextField("Input one", text: .constant(""))
                .layoutValue(key: AppleUIChildWeight.self, value: 1e308)
                .background(GeometryReader { proxy in Color.clear.onAppear { frames.values["input1"] = proxy.frame(in: .named("layout-probe")) } })
            TextField("Input two", text: .constant(""))
                .layoutValue(key: AppleUIChildWeight.self, value: 1e308)
                .background(GeometryReader { proxy in Color.clear.onAppear { frames.values["input2"] = proxy.frame(in: .named("layout-probe")) } })
        }.frame(width: 320).coordinateSpace(name: "layout-probe")
        let host = NSHostingView(rootView: root)
        host.frame = NSRect(x: 0, y: 0, width: 320, height: 600)
        let window = NSWindow(contentRect: host.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        host.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(50))
        host.layoutSubtreeIfNeeded()
        XCTAssertEqual(frames.values.count, 3)
        for (name, frame) in frames.values {
            XCTAssertTrue(frame.width.isFinite && frame.height.isFinite, name)
            XCTAssertGreaterThanOrEqual(frame.minX, -0.5, name)
            XCTAssertLessThanOrEqual(frame.maxX, 320.5, name)
        }
        XCTAssertGreaterThan(try XCTUnwrap(frames.values["text"]).height, 30)
    }

    func testNativeSurfaceMountDeltaAndIMEFocusAtIncreasingNodeCounts() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        for count in [10, 100, 1000] {
            let fixture = MacA2UIRenderFixture(nodeCount: count)
            let start = ProcessInfo.processInfo.systemUptime
            let host = NSHostingView(rootView: MacA2UIRenderProbe(fixture: fixture))
            host.frame = NSRect(x: 0, y: 0, width: 700, height: 600)
            let window = NSWindow(contentRect: host.bounds, styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
            window.makeKeyAndOrderFront(nil)
            host.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(20))
            host.layoutSubtreeIfNeeded()
            let mount = (ProcessInfo.processInfo.systemUptime - start) * 1000
            func input(_ view: NSView) -> NSTextField? {
                if let field = view as? NSTextField, field.placeholderString == "IME probe" { return field }
                return view.subviews.lazy.compactMap { input($0) }.first
            }
            let field = try XCTUnwrap(input(host))
            field.selectText(nil)
            let editor = try XCTUnwrap(field.currentEditor() as? NSTextView)
            for composition in ["中文", "にほんご", "한국어"] {
                editor.setMarkedText(composition, selectedRange: NSRange(location: composition.utf16.count, length: 0), replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
                XCTAssertTrue(editor.hasMarkedText())
                let before = fixture.snapshot
                let changed = AppleUINode<MacCanonicalJSONValue>(instance_id: "text-0", component_id: "text-0", component: "Text", props: ["text": .string(composition)], slots: [:], bindings: [:], actions: [:], validation_errors: [])
                let delta = AppleUIChangeSet(identity: before.identity, base_revision: before.revision, revision: before.revision + 1, catalog_digest: before.catalog_digest, phase: .ready, root_instance_id: "root", upserted_nodes: [changed], removed_instance_ids: [], summary: "probe")
                let tick = ProcessInfo.processInfo.systemUptime
                fixture.snapshot = try XCTUnwrap(AppleUISurfaceUpdate.changes(delta).applying(to: before, identity: before.identity))
                try await Task.sleep(for: .milliseconds(20))
                host.layoutSubtreeIfNeeded()
                let elapsed = (ProcessInfo.processInfo.systemUptime - tick) * 1000
                XCTAssertTrue(input(host) === field, "Delta must preserve the native input instance")
                XCTAssertTrue(field.currentEditor() === editor)
                XCTAssertTrue(editor.hasMarkedText())
                XCTAssertEqual(editor.string, composition)
                print("A2UI_NATIVE_METRIC nodes=\(count) mount_ms=\(mount) delta_ms=\(elapsed) ime=\(composition) scheduling_wait_ms=20")
                editor.unmarkText()
            }
            window.orderOut(nil); window.contentView = nil; window.close()
        }
    }

    func testSurfaceAccessibilityIdentityIsCompactAndIncludesEveryOwnerField() throws {
        let raw = #"{"owner":{"authority_id":"store","conversation_id":"conversation","participant_id":"assistant","context_id":"context","originating_run_id":"run","message_id":"message","part_id":"tool-part"},"surface_id":"form","generation":1}"#
        func identity(_ json: String) throws -> String {
            try JSONDecoder().decode(AppleUISurfaceIdentity.self, from: Data(json.utf8)).accessibilityIdentifier
        }
        let baseline = try identity(raw)
        XCTAssertEqual(baseline, try identity(raw))
        XCTAssertEqual(baseline.count, "a2ui.surface.".count + 64)
        for value in ["store", "conversation", "assistant", "context", "run", "message", "tool-part", "form"] {
            XCTAssertNotEqual(baseline, try identity(raw.replacingOccurrences(of: "\"" + value + "\"", with: "\"changed-" + value + "\"")))
        }
        XCTAssertNotEqual(baseline, try identity(raw.replacingOccurrences(of: "\"generation\":1", with: "\"generation\":2")))
    }

    func testQueueCancellationCompletionPreservesAnotherConversationAndNewerReceipt() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("queue.db"))
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        let target = MacRemoteSubmissionTarget(configuration: .init(baseURL: URL(string: "https://fixture.invalid")!,
            token: "fixture", expectedHostID: "host"), clientID: "client")
        func receipt(_ conversation: String, _ command: String) -> MacSourceSubmissionReceipt {
            .init(protocol_version: 1, identity: .init(source_host_id: "host", client_id: "client",
                conversation_id: conversation, submission_id: command, user_message_id: "user:" + command,
                assistant_message_id: "assistant:" + command), request_digest: "digest:" + command,
                phase: .rejected, admission: nil, rejection_code: "fixture")
        }
        model.selectedConversationID = "A"
        model.recordRejectedUIQueue(conversationID: "A", target: target, receipt: receipt("A", "first"))
        let completeA = try XCTUnwrap(model.uiQueueCancellationCompletion())
        model.selectedConversationID = "B"
        model.recordRejectedUIQueue(conversationID: "B", target: target, receipt: receipt("B", "second"))
        let errorB = model.remoteRecoveryError
        completeA()
        XCTAssertTrue(model.canCancelRejectedUIQueue)
        XCTAssertEqual(model.remoteRecoveryError, errorB)
        model.selectedConversationID = "A"
        model.recordRejectedUIQueue(conversationID: "A", target: target, receipt: receipt("A", "newer"))
        completeA()
        XCTAssertTrue(model.canCancelRejectedUIQueue, "A duplicate old completion cannot consume the newer receipt")
        let completeNew = try XCTUnwrap(model.uiQueueCancellationCompletion())
        completeNew()
        XCTAssertFalse(model.canCancelRejectedUIQueue)
        XCTAssertNil(model.remoteRecoveryError)
    }

    func testQueuedUIActionDisplaysSummaryWithoutRewritingCorePayload() throws {
        let raw = #"{"type":"submission","submission":{"type":"user_input","submission_id":"command","items":[{"type":"ui_action","id":"user","action":{"summary":"Submitted form","envelope":{"name":"private-value"}}}]}}"#
        let decoded = try JSONDecoder().decode(AppleUIQueuedSubmission.self, from: Data(raw.utf8))
        XCTAssertEqual(try decoded.item(commandID: "command").action.summary, "Submitted form")
        XCTAssertEqual(try decoded.item(commandID: "command").id, "user")
        XCTAssertThrowsError(try decoded.item(commandID: "foreign"))
        let old = raw.replacingOccurrences(of: "ui_action", with: "message")
        XCTAssertThrowsError(try JSONDecoder().decode(AppleUIQueuedSubmission.self, from: Data(old.utf8)).item(commandID: "command"))
    }

    func testUIQueueDispatchRequiresOriginalOwnerAndCurrentStateShape() throws {
        let raw = #"{"head":{"command_id":"submission","kind":"user_input","user_message_id":"ui:submission:user","lease_owner":null,"lease_until_ms":null},"dispatch":{"protocol_version":1,"identity":{"source_host_id":"host","client_id":"client","conversation_id":"conversation","submission_id":"submission","user_message_id":"ui:submission:user","assistant_message_id":"assistant"},"request_digest":"digest","phase":"prepared","admission":null,"rejection_code":null}}"#
        func decode(_ value: String) throws -> MacUIQueueState {
            try JSONDecoder().decode(MacUIQueueState.self, from: Data(value.utf8))
        }
        try decode(raw).validate(hostID: "host", clientID: "client", conversationID: "conversation")
        XCTAssertThrowsError(try decode(raw).validate(hostID: "changed", clientID: "client", conversationID: "conversation"))
        XCTAssertThrowsError(try decode(raw).validate(hostID: "host", clientID: "changed", conversationID: "conversation"))
        XCTAssertThrowsError(try decode(raw.replacingOccurrences(of: "\"request_digest\":\"digest\"", with: "\"request_digest\":\"\"")).validate(hostID: "host", clientID: "client", conversationID: "conversation"))
        XCTAssertThrowsError(try decode("{}"))
        XCTAssertThrowsError(try decode("{\"command_id\":\"submission\"}"))
        try decode("{\"head\":null,\"dispatch\":null}").validate(hostID: "host", clientID: "client", conversationID: "conversation")
    }

    func testUIActionRetainsEnvelopeAsDataAndDisplaysOnlySummary() throws {
        let payload = #"{"kind":"ui_action","part_id":"action:submission","revision":9,"surface":{"identity":{"owner":{"authority_id":"store","conversation_id":"conversation","participant_id":"assistant","context_id":"context","originating_run_id":"run","message_id":"message","part_id":"tool-part"},"surface_id":"form","generation":1},"revision":3},"envelope":{"secret_form_value":"never-render-this-envelope"},"summary":"已提交表单"}"#
        let part = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Part.self, from: Data(payload.utf8))
        guard case .uiAction(let value) = part else { return XCTFail("Expected typed UI action") }
        XCTAssertEqual(value.partID, "action:submission")
        XCTAssertEqual(value.summary, "已提交表单")
        XCTAssertEqual(value.surface.revision, 3)
        XCTAssertFalse(value.summary.contains("never-render-this-envelope"))
        let messageJSON = "{\"message_id\":\"action-message\",\"role\":\"user\",\"state\":\"completed\",\"revision\":9,\"created_at_ms\":1,\"parts\":[" + payload + "]}"
        let message = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Message.self, from: Data(messageJSON.utf8))
        XCTAssertEqual(try MacCanonicalConversationRepository.projectMessage(message).text, "已提交表单")
    }

    func testUISurfacePartPreservesIndependentRevisionsAndToolProvenance() throws {
        let payload = #"{"kind":"ui_surface","part_id":"ui-surface:hash","revision":42,"surface":{"identity":{"owner":{"authority_id":"store","conversation_id":"conversation","participant_id":"assistant","context_id":"context","originating_run_id":"run","message_id":"message","part_id":"tool-part"},"surface_id":"form","generation":1},"revision":3},"summary":"Form"}"#
        let part = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Part.self, from: Data(payload.utf8))
        guard case .uiSurface(let value) = part else { return XCTFail("Expected typed Surface Part") }
        XCTAssertEqual(value.partID, "ui-surface:hash")
        XCTAssertEqual(value.revision, 42)
        XCTAssertEqual(value.surface.revision, 3)
        XCTAssertEqual(value.surface.identity.owner.part_id, "tool-part")
        XCTAssertThrowsError(try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Part.self,
            from: Data(payload.replacingOccurrences(of: "\"revision\":3", with: "\"revision\":0").utf8)))
    }

    func testUISurfaceJournalMetadataIsTypedAndPreservesExactVersion() throws {
        let payload = #"{"kind":"ui_surface","data":{"surface":{"identity":{"owner":{"authority_id":"store","conversation_id":"conversation","participant_id":"assistant","context_id":"context","originating_run_id":"run","message_id":"message","part_id":"tool-part"},"surface_id":"form","generation":1},"revision":3},"summary":"Form"}}"#
        let metadata = try JSONDecoder().decode(MacAssistantJournalMetadata.self, from: Data(payload.utf8))
        guard case .uiSurface(let value) = metadata else { return XCTFail("Expected Surface metadata") }
        XCTAssertEqual(value.surface.revision, 3)
        XCTAssertEqual(value.surface.identity.owner.message_id, "message")
        XCTAssertEqual(value.summary, "Form")
    }

    func testHistoricalSurfaceRequiresCompleteCanonicalSupportedReference() throws {
        let raw = #"{"owner":{},"canonical_revision":42,"schema":"message_part_view","canonical_kind":"ui_surface","binding":"canonical","status":{"state":"supported"},"fields":{"kind":{"availability":"complete","value":"ui_surface"},"part_id":{"availability":"complete","value":"ui-surface:hash"},"revision":{"availability":"complete","value":42},"surface":{"availability":"complete","value":{"identity":{"owner":{"authority_id":"store","conversation_id":"conversation","participant_id":"assistant","context_id":"context","originating_run_id":"run","message_id":"message","part_id":"tool"},"surface_id":"form","generation":1},"revision":3}},"summary":{"availability":"complete","value":"Form"}}}"#
        func decode(_ json: String) throws -> MacCanonicalConversationProjectionV1.HistoricalProjection {
            try JSONDecoder().decode(MacCanonicalConversationProjectionV1.HistoricalProjection.self, from: Data(json.utf8))
        }
        XCTAssertEqual(try decode(raw).uiSurfaceReference?.revision, 3)
        var incomplete = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any])
        let completeFields = try XCTUnwrap(incomplete["fields"] as? [String: Any])
        for key in ["kind", "part_id", "revision", "surface", "summary"] {
            var missingFields = completeFields
            missingFields.removeValue(forKey: key)
            incomplete["fields"] = missingFields
            let encoded = try JSONSerialization.data(withJSONObject: incomplete)
            XCTAssertNil(try decode(String(decoding: encoded, as: UTF8.self)).uiSurfaceReference)
        }
        for invalid in [
            raw.replacingOccurrences(of: "\"binding\":\"canonical\"", with: "\"binding\":\"untrusted\""),
            raw.replacingOccurrences(of: "\"state\":\"supported\"", with: "\"state\":\"deferred\""),
            raw.replacingOccurrences(of: "\"schema\":\"message_part_view\"", with: "\"schema\":\"agent_part\""),
            raw.replacingOccurrences(of: "\"availability\":\"complete\",\"value\":\"Form\"", with: "\"availability\":\"deferred\",\"locator\":{}"),
        ] {
            XCTAssertNil(try decode(invalid).uiSurfaceReference)
        }
    }

    func testSurfaceDeltaRequiresExactBaseAndRetainsUntouchedNodes() throws {
        let owner = AppleUISurfaceOwner(authority_id: "store", conversation_id: "conversation", participant_id: "agent",
            context_id: "context", originating_run_id: "run", message_id: "message", part_id: "tool")
        let identity = AppleUISurfaceIdentity(owner: owner, surface_id: "form", generation: 1)
        let old = AppleUINode<MacCanonicalJSONValue>(instance_id: "old", component_id: "old", component: "Text", props: [:], slots: [:], bindings: [:], actions: [:], validation_errors: [])
        let retained = AppleUINode<MacCanonicalJSONValue>(instance_id: "keep", component_id: "keep", component: "Text", props: [:], slots: [:], bindings: [:], actions: [:], validation_errors: [])
        let base = AppleUISnapshot(identity: identity, revision: 2, catalog_id: "catalog", catalog_digest: "digest",
            phase: .ready, root_instance_id: "keep", nodes: ["old": old, "keep": retained], summary: "Old")
        let change = AppleUIChangeSet<MacCanonicalJSONValue>(identity: identity, base_revision: 2, revision: 4, catalog_digest: "digest",
            phase: .ready, root_instance_id: "keep", upserted_nodes: [], removed_instance_ids: ["old"], summary: "New")
        let delta = AppleUISurfaceUpdate<MacCanonicalJSONValue>.changes(change)
        let result = try XCTUnwrap(delta.applying(to: base, identity: identity))
        XCTAssertEqual(result.revision, 4)
        XCTAssertEqual(result.nodes["keep"], retained)
        XCTAssertNil(result.nodes["old"])
        XCTAssertNotNil(base.nodes["old"])
        XCTAssertThrowsError(try delta.applying(to: nil, identity: identity))
        XCTAssertThrowsError(try delta.applying(to: result, identity: identity))
    }

    func testUIIntentRecoveryPreservesOriginalRequestAndSource() throws {
        var storage: [String: (json: String, revision: Int64)] = [:]
        func journal() -> AppleUIIntentJournal<MacCanonicalJSONValue> {
            AppleUIIntentJournal(read: { storage[$0] }, write: { key, value, expected in
                guard storage[key]?.revision == expected else { throw NSError(domain: "CAS", code: 1) }
                storage[key] = (value, (expected ?? 0) + 1)
            })
        }
        let identity = AppleUISurfaceIdentity(owner: AppleUISurfaceOwner(authority_id: "store", conversation_id: "conversation",
            participant_id: "agent", context_id: "context", originating_run_id: "run", message_id: "message", part_id: "tool"),
            surface_id: "form", generation: 1)
        let source = AppleUIIntentSource(hostID: "host", address: "https://host.example", certificate: "certificate", clientID: "client")
        let input = AppleUISubmission<MacCanonicalJSONValue>(interaction: AppleUIInteraction(identity: identity, expected_surface_revision: 3,
            edits: [], action: nil), submission_id: "original", submitted_at_ms: 123)
        try journal().stage(input, source: source)
        let recovered = try XCTUnwrap(journal().pending(identity: identity, source: source))
        XCTAssertEqual(recovered.submission_id, "original")
        XCTAssertEqual(recovered.submitted_at_ms, 123)
        XCTAssertEqual(recovered.interaction.expected_surface_revision, 3)
        let replacement = AppleUISubmission(interaction: input.interaction, submission_id: "replacement", submitted_at_ms: 456)
        XCTAssertThrowsError(try journal().stage(replacement, source: source))
        XCTAssertThrowsError(try journal().pending(identity: identity,
            source: AppleUIIntentSource(hostID: "other", address: source.address, certificate: source.certificate, clientID: source.clientID)))
        try journal().retire(input, source: source)
        XCTAssertNil(try journal().pending(identity: identity, source: source))
        try journal().stage(replacement, source: source)
        XCTAssertEqual(try journal().pending(identity: identity, source: source)?.submission_id, "replacement")
    }

    func testUIIntentRestoresFromColdCanonicalSettingsStore() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("a2ui-intent-\(UUID())", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("state.db")
        func journal() throws -> AppleUIIntentJournal<MacCanonicalJSONValue> {
            let runtime = try MacApplicationPort(databaseURL: database)
            return AppleUIIntentJournal(read: { key in
                try runtime.productSetting(key: key).map { ($0.valueJSON, $0.revision) }
            }, write: { key, value, expected in
                _ = try runtime.putProductSettingChecked(key: key, valueJSON: value, expectedRevision: expected, updatedAtMs: 123)
            })
        }
        let identity = AppleUISurfaceIdentity(owner: AppleUISurfaceOwner(authority_id: "store", conversation_id: "conversation",
            participant_id: "agent", context_id: "context", originating_run_id: "run", message_id: "message", part_id: "tool"),
            surface_id: "form", generation: 1)
        let source = AppleUIIntentSource(hostID: "host", address: "https://host.example", certificate: "cert", clientID: "client")
        let input = AppleUISubmission<MacCanonicalJSONValue>(interaction: AppleUIInteraction(identity: identity,
            expected_surface_revision: 3, edits: [.set(instanceID: "field", binding: "value", value: .string("unsent draft"))], action: nil),
            submission_id: "original", submitted_at_ms: 123)
        try journal().stage(input, source: source)
        let cold = try journal()
        let restored = try XCTUnwrap(cold.pending(identity: identity, source: source))
        let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys
        XCTAssertEqual(try encoder.encode(restored), try encoder.encode(input))
        try cold.retire(input, source: source)
        XCTAssertNil(try journal().pending(identity: identity, source: source))
    }

    func testUIIntentStorageFailuresNeverLoseOriginalSubmission() throws {
        var storage: [String: (json: String, revision: Int64)] = [:]
        var failWrite = true
        let journal = AppleUIIntentJournal<MacCanonicalJSONValue>(read: { storage[$0] }, write: { key, value, expected in
            guard !failWrite, storage[key]?.revision == expected else { throw NSError(domain: "CAS", code: 1) }
            storage[key] = (value, (expected ?? 0) + 1)
        })
        let identity = AppleUISurfaceIdentity(owner: AppleUISurfaceOwner(authority_id: "store", conversation_id: "conversation",
            participant_id: "agent", context_id: "context", originating_run_id: "run", message_id: "message", part_id: "tool"),
            surface_id: "form", generation: 1)
        let source = AppleUIIntentSource(hostID: "host", address: "https://host.example", certificate: "cert", clientID: "client")
        let input = AppleUISubmission<MacCanonicalJSONValue>(interaction: AppleUIInteraction(identity: identity,
            expected_surface_revision: 3, edits: [], action: nil), submission_id: "original", submitted_at_ms: 123)
        XCTAssertThrowsError(try journal.stage(input, source: source))
        XCTAssertTrue(storage.isEmpty)
        failWrite = false
        try journal.stage(input, source: source)
        let saved = storage
        failWrite = true
        // Replaying the exact durable request needs no new write or identifier.
        XCTAssertNoThrow(try journal.stage(input, source: source))
        XCTAssertThrowsError(try journal.retire(input, source: source))
        XCTAssertEqual(try journal.pending(identity: identity, source: source)?.submission_id, "original")
        XCTAssertEqual(storage.values.first?.revision, saved.values.first?.revision)
        for changedSource in [
            AppleUIIntentSource(hostID: source.hostID, address: "https://other.example", certificate: source.certificate, clientID: source.clientID),
            AppleUIIntentSource(hostID: source.hostID, address: source.address, certificate: "new-cert", clientID: source.clientID),
            AppleUIIntentSource(hostID: source.hostID, address: source.address, certificate: source.certificate, clientID: "new-client"),
        ] {
            XCTAssertThrowsError(try journal.pending(identity: identity, source: changedSource))
        }
        failWrite = false
        try journal.retire(input, source: source)
        XCTAssertNil(try journal.pending(identity: identity, source: source))
    }

    func testCodeCellProvenanceRequiresExactOwnerAndRejectsUnknownFields() throws {
        let cell: [String: Any] = [
            "host_id": "host", "conversation_id": "conversation", "turn_id": "turn",
            "parent_call_id": "parent", "cell_id": "cell", "generation": "generation",
        ]
        func decode(_ value: [String: Any]) throws -> MacCanonicalConversationProjectionV1.ToolExecutionSource {
            try JSONDecoder().decode(MacCanonicalConversationProjectionV1.ToolExecutionSource.self,
                from: JSONSerialization.data(withJSONObject: value))
        }
        XCTAssertEqual(try decode(["kind": "model"]).kind, .model)
        let source = try decode(["kind": "code_cell", "cell": cell])
        XCTAssertEqual(source.cell?.conversationID, "conversation")
        XCTAssertEqual(source.cell?.parentCallID, "parent")
        XCTAssertEqual(source.cell?.generation, "generation")
        for invalid in [
            [:], ["kind": "unknown"], ["kind": "model", "cell": NSNull()],
            ["kind": "model", "permission": "allow"], ["kind": "code_cell"],
            ["kind": "code_cell", "cell": cell, "permission": "allow"],
        ] as [[String: Any]] {
            XCTAssertThrowsError(try decode(invalid))
        }
        var extra = cell
        extra["alias"] = "owner"
        XCTAssertThrowsError(try decode(["kind": "code_cell", "cell": extra]))
        for key in cell.keys {
            var missing = cell
            missing.removeValue(forKey: key)
            XCTAssertThrowsError(try decode(["kind": "code_cell", "cell": missing]))
            for invalid in ["", "   ", "owner\n", String(repeating: "界", count: 171)] {
                var malformed = cell
                malformed[key] = invalid
                XCTAssertThrowsError(try decode(["kind": "code_cell", "cell": malformed]))
            }
        }
    }

    private let conversation = "rich-conversation"
    private let run = "rich-run"
    private var assistant: String { "\(run):assistant" }

    private func tool(output: Any? = nil) -> [String: Any] {
        var value: [String: Any] = [
            "kind": "tool", "execution_source": ["kind": "model"], "part_id": "rich-tool", "revision": 2,
            "tool_call_id": "rich-call", "capability_id": "write_file", "state": "completed",
            "title": "创建文件 — 标题", "summary": "", "input": ["path": "/tmp/User Report.txt"],
            "origin": ["kind": "mcp", "provider_id": "provider", "provider_title": "独立提供方", "namespace": "ns"],
            "permission": ["request_id": "approval", "permission_domain": "filesystem", "rationale": "仅展示的理由", "decision": "allow"],
            "output_preview": "独立预览", "full_output": ["message_id": assistant, "part_id": "rich-tool", "tool_call_id": "rich-call"],
            "artifacts": [["uri": "file:///tmp/User%20Report.txt", "name": "报告.txt", "mime_type": "text/plain", "operation": "create"]],
            "started_at_ms": 10, "finished_at_ms": 20, "affected_resources": [] as [String],
        ]
        if let output { value["output"] = output }
        return value
    }

    private func message(parts: [[String: Any]]) -> [String: Any] {
        ["message_id": assistant, "role": "assistant", "state": "running", "revision": 2,
         "created_at_ms": 1, "parts": parts]
    }

    private func decodeMessage(_ value: [String: Any]) throws -> MacCanonicalConversationProjectionV1.Message {
        try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Message.self,
            from: JSONSerialization.data(withJSONObject: value))
    }

    func testCanonicalNullPreviewAndEmptySummaryAreIndependentAndOwnerBound() throws {
        let message = try decodeMessage(self.message(parts: [tool(output: NSNull())]))
        let projected = try MacCanonicalConversationRepository.projectMessage(message)
        let trace = try XCTUnwrap(projected.presentation?.traceItems.first)
        let metadata = try XCTUnwrap(trace.tool)
        XCTAssertEqual(trace.title, "创建文件 — 标题")
        XCTAssertEqual(metadata.summary, "")
        XCTAssertEqual(metadata.output, .null)
        XCTAssertEqual(metadata.outputPreview, "独立预览")
        XCTAssertEqual(metadata.resultText, "null")
        XCTAssertEqual(metadata.origin.providerTitle, "独立提供方")
        XCTAssertEqual(metadata.permission?.decision, "allow")
        XCTAssertEqual(metadata.artifacts.first?.name, "报告.txt")
        XCTAssertEqual(metadata.artifacts.first?.operation, "create")
        XCTAssertEqual(metadata.startedAtMs, 10)
        XCTAssertEqual(metadata.finishedAtMs, 20)
        XCTAssertEqual(metadata.fullOutput?.messageID, assistant)
        XCTAssertEqual(metadata.fullOutput?.partID, "rich-tool")
        XCTAssertEqual(metadata.fullOutput?.toolCallID, "rich-call")
        XCTAssertEqual(trace.affectedFiles, ["/tmp/User Report.txt"])
        XCTAssertEqual(MacToolTraceDisplayPolicy.fullResult(for: trace), "null")
        let absent = try decodeMessage(self.message(parts: [tool()]))
        guard let part = absent.parts.first, case .tool(let absentTool) = part else { return XCTFail("Expected a Tool") }
        XCTAssertNil(absentTool.output)
        var citation: [String: Any] = ["kind": "citation", "part_id": "citation", "revision": 2,
            "citation_id": "actual", "source_id": "source", "title": "", "uri": "https://example.com",
            "origin_tool_call_id": "rich-call", "location": [:]]
        guard case .citation(let emptyLocation) = try XCTUnwrap(decodeMessage(self.message(parts: [citation])).parts.first) else { return XCTFail("Expected citation") }
        XCTAssertNotNil(emptyLocation.location)
        citation["section"] = "inconsistent"
        XCTAssertThrowsError(try decodeMessage(self.message(parts: [citation])))
        citation.removeValue(forKey: "section")
        citation["range_start"] = 7
        XCTAssertThrowsError(try decodeMessage(self.message(parts: [citation])))
        for review in ["pending", "rejected", "interrupted"] {
            let reviewed = try decodeMessage(self.message(parts: [["kind": "text", "part_id": "withheld", "revision": 2,
                "markdown": "PRIVATE-\(review)", "is_final": true, "review": review]]))
            XCTAssertEqual(try MacCanonicalConversationRepository.projectMessage(reviewed).text, "")
        }
        for key in ["message_id", "part_id", "tool_call_id", "path"] {
            var invalid = tool(output: NSNull())
            var owner = try XCTUnwrap(invalid["full_output"] as? [String: Any])
            owner[key] = "/private/other-owner"
            invalid["full_output"] = owner
            XCTAssertThrowsError(try decodeMessage(self.message(parts: [invalid])), key)
        }
    }

    func testCanonicalDecoderPreservesNullMissingAndRejectsMalformedReview() throws {
        for output in [nil, NSNull()] as [Any?] {
            let decoded = try decodeMessage(message(parts: [tool(output: output)]))
            guard case .tool(let value) = try XCTUnwrap(decoded.parts.first) else {
                return XCTFail("Expected typed Tool")
            }
            XCTAssertEqual(value.output, output == nil ? nil : .null)
            let encoded = try JSONSerialization.data(withJSONObject: message(parts: [tool(output: output)]), options: [.sortedKeys])
            let roundtrip = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Message.self, from: encoded)
            XCTAssertEqual(try MacCanonicalConversationRepository.projectMessage(roundtrip).presentation,
                           try MacCanonicalConversationRepository.projectMessage(decoded).presentation)
        }
        for malformed: Any in ["unknown", 1, ["status": "approved"]] {
            XCTAssertThrowsError(try decodeMessage(message(parts: [["kind": "text", "part_id": "review",
                "revision": 1, "markdown": "private", "is_final": true, "review": malformed]])))
        }
    }

    func testRichPublicDTOAndMacPresenterRetainEverySemanticField() throws {
        let usage: [String: Any] = ["record_id": "record", "component_id": "component",
            "scope": "global", "namespace": ["kind": "workspace"], "provenance_kind": "tool_result",
            "requires_current_state_verification": true, "truncated": true, "supersedes": ["older"],
            "usage_count": 7, "last_used_at_epoch_ms": 31]
        var parts: [[String: Any]] = [tool(output: NSNull()),
            ["kind": "reasoning", "part_id": "rich-reasoning", "revision": 2, "state": "completed",
             "title": "推理标题 α", "summary": "独立推理正文"],
            ["kind": "source", "part_id": "rich-source", "revision": 2, "source_id": "source",
             "title": "Evidence", "uri": "https://example.com/reference", "evidence": ["kind": "memory", "usage": usage]],
            ["kind": "citation", "part_id": "citation-part", "revision": 2, "source_id": "source",
             "citation_id": "real-citation-id", "title": "Citation", "uri": "https://example.com/reference",
             "mime_type": "application/pdf", "origin_tool_call_id": "rich-call",
             "location": ["page": 3, "section": "方法 α"], "page": 3, "section": "方法 α", "range_start": 7, "range_end": 12],
            ["kind": "citation", "part_id": "empty-location-part", "revision": 2, "source_id": "source",
             "citation_id": "empty-location-id", "title": "", "uri": "https://example.com/reference",
             "origin_tool_call_id": "rich-call", "location": [:]],
            ["kind": "attachment", "part_id": "rich-artifact", "attachment_id": "artifact", "revision": 2,
             "name": "Created.pdf", "uri": "file:///tmp/Created.pdf", "mime_type": "application/pdf", "operation": "create"],
            ["kind": "text", "part_id": "public-text", "revision": 2, "markdown": "Visible answer", "is_final": true, "review": "approved"],
        ]
        for review in ["pending", "rejected", "interrupted"] {
            parts.append(["kind": "text", "part_id": "text-\(review)", "revision": 2,
                          "markdown": "", "is_final": true, "review": review])
        }
        let canonical = try decodeMessage(message(parts: parts))
        let projected = try MacCanonicalConversationRepository.projectMessage(canonical)
        XCTAssertEqual(projected.text, "Visible answer")
        let presentation = try XCTUnwrap(projected.presentation)
        let tool = try XCTUnwrap(presentation.traceItems.first { $0.externalID == "rich-tool" })
        XCTAssertEqual(tool.tool?.output, .null)
        XCTAssertEqual(tool.tool?.summary, "")
        XCTAssertEqual(tool.tool?.outputPreview, "独立预览")
        XCTAssertEqual(tool.tool?.permission?.rationale, "仅展示的理由")
        XCTAssertEqual(tool.tool?.artifacts.first?.operation, "create")
        XCTAssertEqual(presentation.traceItems.first { $0.externalID == "rich-reasoning" }?.title, "推理标题 α")
        let citation = try XCTUnwrap(presentation.renderedSourceReferences.first { $0.citationID == "real-citation-id" })
        XCTAssertEqual(citation.id, "real-citation-id")
        XCTAssertEqual(citation.mimeType, "application/pdf")
        XCTAssertEqual(citation.section, "方法 α")
        XCTAssertEqual(citation.rangeStart, 7)
        XCTAssertEqual(citation.rangeEnd, 12)
        XCTAssertEqual(citation.originToolCallID, "rich-call")
        XCTAssertEqual(citation.memoryUsage?.usageCount, 7)
        XCTAssertEqual(citation.memoryUsage?.supersedes, ["older"])
        XCTAssertEqual(citation.memoryUsage?.requiresCurrentStateVerification, true)
        XCTAssertNotNil(presentation.renderedSourceReferences.first { $0.citationID == "empty-location-id" }?.location)
        XCTAssertEqual(projected.attachmentMetadata?.first?.operation, "create")
    }

    func testRealSourceJournalAdmissionTerminalAndColdMacConsumption() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("nola-public-parity-\(UUID())")
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("store.db")
        let port = try MacApplicationPort(databaseURL: database)
        _ = try port.prepareFixtureRemoteSubmission(.init(conversationID: conversation, conversationTitle: "Rich",
            submissionID: run, sourceHostID: "fixture-host", clientID: "fixture-client", userMessageID: "rich-user", userPartID: "rich-user:text",
            userText: "Original request", userContentJSON: nil, assistantMessageID: assistant,
            assistantPartID: "\(assistant):text", contextCheckpointID: nil, createdAtMs: 1))
        let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation,
            runID: run, assistantMessageID: assistant)
        let first = try source.append("Visible answer — ")
        try source.ingest(into: port, through: first)
        let observed = MacCanonicalConversationRepository(store: port).makeWindowModel()
        try await observed.select(conversationID: conversation)
        XCTAssertEqual(observed.messages.last?.journalReference?.cursor, first)
        let originalReference = try XCTUnwrap(observed.messages.last?.journalReference)
        let originalParts = try await MacJournalTextRead.parts(originalReference)
        let second = try source.append("第二段🙂")
        try source.ingest(into: port, through: second)
        let advanced = try await observed.refreshIfChanged()
        XCTAssertFalse(advanced, "Body-only journal commits must not wake the row-window subscription")
        XCTAssertEqual(observed.messages.last?.journalReference?.cursor, first)
        // Consume the actual committed watermark through the same incremental
        // reader as the live message view, without reloading transcript rows.
        let updatedReference = MacJournalMessageReference(databaseURL: database, cursor: second)
        let updatedParts = try await MacJournalTextRead.updatedParts(updatedReference,
            from: originalReference, existing: originalParts)
        let textPart = try XCTUnwrap(updatedParts.first {
            if case .text(let text) = $0.metadata { return text.visibility == "public" }
            return false
        })
        let body = try XCTUnwrap(textPart.bodies.first { $0.field == "markdown" })
        let updatedText = try await MacJournalTextRead.completeSmallText(updatedReference,
            partID: textPart.part_id, content: body)
        XCTAssertEqual(updatedText, "Visible answer — 第二段🙂")
        _ = try source.commitTerminal(into: port)
        let terminalAdvanced = try await observed.refreshIfChanged()
        XCTAssertTrue(terminalAdvanced, "Terminal admission must still wake the structural window")
        var firstCanonical: MacCanonicalConversationProjectionV1.Message?
        for application in [port, try MacApplicationPort(databaseURL: database)] {
            let window = try application.loadConversationWindow(conversation)
            let projection = window.projection
            let canonical = try XCTUnwrap(projection.messages.first { $0.messageID == assistant })
            let messages = try MacCanonicalConversationRepository.projectMessages(window, databaseURL: database)
            let reference = try XCTUnwrap(messages.first { $0.role == .assistant }?.journalReference)
            let markdown = try await MacJournalTextRead.markdown(reference, historicalFallback: "")
            XCTAssertEqual(markdown, "Visible answer — 第二段🙂")
            if let firstCanonical { XCTAssertEqual(canonical, firstCanonical) }
            else { firstCanonical = canonical }
            XCTAssertNil(projection.activeRun)
            XCTAssertEqual(projection.messages.filter { $0.role == .user }.count, 1)
            XCTAssertEqual(projection.messages.filter { $0.role == .assistant }.count, 1)
        }
    }
}

@MainActor
private final class MacA2UIRenderFixture: ObservableObject {
    @Published var snapshot: AppleUISnapshot<MacCanonicalJSONValue>
    init(nodeCount: Int) {
        let owner = AppleUISurfaceOwner(authority_id: "test", conversation_id: UUID().uuidString, participant_id: "agent", context_id: "context", originating_run_id: "run", message_id: "message", part_id: "part")
        var nodes: [String: AppleUINode<MacCanonicalJSONValue>] = [:]
        let texts = (0..<(nodeCount - 2)).map { "text-\($0)" }
        nodes["root"] = .init(instance_id: "root", component_id: "root", component: "Column", props: [:], slots: ["children": ["input"] + texts], bindings: [:], actions: [:], validation_errors: [])
        nodes["input"] = .init(instance_id: "input", component_id: "input", component: "TextField", props: ["label": .string("IME probe"), "value": .string("")], slots: [:], bindings: ["value": "/name"], actions: [:], validation_errors: [])
        for id in texts { nodes[id] = .init(instance_id: id, component_id: id, component: "Text", props: ["text": .string(id)], slots: [:], bindings: [:], actions: [:], validation_errors: []) }
        snapshot = .init(identity: .init(owner: owner, surface_id: "probe", generation: 1), revision: 1, catalog_id: "fixture", catalog_digest: "fixture", phase: .ready, root_instance_id: "root", nodes: nodes, summary: "probe")
    }
}

@MainActor
private struct MacA2UIRenderProbe: View {
    @ObservedObject var fixture: MacA2UIRenderFixture
    var body: some View {
        ScrollView {
            AppleUISurfaceView(snapshot: fixture.snapshot, submit: { _ in .rejected("No model or submission in renderer benchmark") }, accepted: { _ in })
        }
    }
}

@MainActor
private final class MacA2UILayoutFrames {
    var values: [String: CGRect] = [:]
}
