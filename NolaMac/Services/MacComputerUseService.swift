// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Executes macOS Accessibility and pointer operations inside the signed Nola app process.
// - Security: The Agent Host supplies validated tool arguments; this service never launches helper processes.

import AppKit
import ApplicationServices
import CoreGraphics
import Foundation
import ScreenCaptureKit

@MainActor
enum MacComputerUseService {
    static let accessibilityDescendantAttributeNames = [
        kAXFocusedWindowAttribute,
        kAXWindowsAttribute,
        kAXChildrenAttribute,
        kAXVisibleChildrenAttribute,
        kAXContentsAttribute,
        kAXRowsAttribute,
        kAXColumnsAttribute,
        kAXFocusedUIElementAttribute,
    ]

    static func responseData(
        requestID: String,
        toolName: String,
        arguments: [String: Any]
    ) async -> Data {
        let result = await execute(toolName: toolName, arguments: arguments)
        let response: [String: Any] = [
            "type": "device_tool_result",
            "request_id": requestID,
            "output": result.output,
            "display": result.display,
            "is_error": result.isError,
        ]
        return (try? JSONSerialization.data(withJSONObject: response, options: [.sortedKeys]))
            ?? Data("{\"type\":\"device_tool_result\",\"is_error\":true}".utf8)
    }

    private struct Result {
        let output: Any
        let display: String
        let isError: Bool

        static func ok(_ output: Any, _ display: String) -> Result {
            Result(output: output, display: display, isError: false)
        }

        static func failure(_ message: String) -> Result {
            Result(output: ["error": message], display: message, isError: true)
        }
    }

    private struct Node {
        let element: AXUIElement
        let depth: Int
        let payload: [String: Any]
    }

    private static func execute(toolName: String, arguments: [String: Any]) async -> Result {
        switch toolName {
        case "computer_status":
            return status()
        case "computer_list_displays":
            return listDisplays()
        case "computer_screenshot":
            return await screenshot(arguments)
        case "computer_get_clipboard":
            return getClipboard()
        case "computer_set_clipboard":
            return setClipboard(arguments)
        case "computer_type_text":
            return await typeText(arguments)
        case "computer_press_key":
            return pressKey(arguments)
        case "computer_hotkey":
            return hotkey(arguments)
        case "computer_key_down":
            return keyTransition(arguments, down: true)
        case "computer_key_up":
            return keyTransition(arguments, down: false)
        case "computer_copy_selection":
            return await copySelection(arguments)
        case "computer_get_position":
            let point = CGEvent(source: nil)?.location ?? .zero
            return .ok(["x": point.x, "y": point.y], String(localized: "Pointer position read"))
        case "computer_move_mouse":
            return move(arguments)
        case "computer_mouse_down":
            return mouseButton(arguments, down: true)
        case "computer_mouse_up":
            return mouseButton(arguments, down: false)
        case "computer_click", "computer_press_at":
            return click(arguments)
        case "computer_drag":
            return drag(arguments)
        case "computer_scroll":
            return scroll(arguments)
        default:
            return executeAccessibility(toolName: toolName, arguments: arguments)
        }
    }

    private static func status() -> Result {
        let screen = NSScreen.main?.frame ?? .zero
        let accessibility = MacSystemPermissionProbe.accessibilitySnapshot()
        let screenRecording = MacSystemPermissionProbe.screenRecordingSnapshot()
        let targetApplication = MacAccessibilityApplicationResolver.defaultTargetApplication()
        return .ok([
            "available": true,
            "executor": "nola_mac_app",
            "accessibility_permission": accessibility.isAuthorized,
            "accessibility_system_trusted": accessibility.systemTrusted,
            "accessibility_event_posting_authorized": accessibility.eventPostingAuthorized,
            "accessibility_system_wide_readable": accessibility.systemWideReadable,
            "accessibility_focused_element_readable": accessibility.focusedElementReadable,
            "accessibility_tree_readable": accessibility.treeReadable,
            "accessibility_runtime_observed": accessibility.runtimeObserved,
            "screen_recording_permission": screenRecording.isAuthorized,
            "screen_recording_system_preflight": screenRecording.systemPreflight,
            "screen_recording_runtime_observed": screenRecording.runtimeCaptureSucceeded,
            "permission_subject": MacSystemPermissionProbe.permissionSubject(),
            "permission_state_is_live": true,
            "default_target_application": targetApplication?.localizedName ?? "",
            "default_target_bundle_id": targetApplication?.bundleIdentifier ?? "",
            "screen": ["x": screen.minX, "y": screen.minY, "width": screen.width, "height": screen.height],
        ], "Mac computer status")
    }

    private static func listDisplays() -> Result {
        let screenRecording = MacSystemPermissionProbe.screenRecordingSnapshot()
        let displays = NSScreen.screens.enumerated().map { index, screen in
            let id = (screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0
            return [
                "index": index + 1,
                "display_id": Int(id),
                "x": screen.frame.minX,
                "y": screen.frame.minY,
                "width": screen.frame.width,
                "height": screen.frame.height,
                "main": id == CGMainDisplayID(),
            ] as [String: Any]
        }
        return .ok([
            "displays": displays,
            "count": displays.count,
            "screen_recording_permission": screenRecording.isAuthorized,
            "screen_recording_system_preflight": screenRecording.systemPreflight,
            "screen_recording_runtime_observed": screenRecording.runtimeCaptureSucceeded,
            "permission_subject": MacSystemPermissionProbe.permissionSubject(),
        ], "Mac displays listed")
    }

    private static func screenshot(_ arguments: [String: Any]) async -> Result {
        if MacProcessRuntimePolicy.isRunningTests,
           !MacProcessRuntimePolicy.allowsSystemPermissionPrompts {
            return .failure(
                "Screen capture is disabled in the default XCTest runtime. "
                    + "Set NOLA_REQUIRE_MAC_SYSTEM_PERMISSIONS=1 for an explicit system-permission integration run."
            )
        }
        do {
            let shareable = try await SCShareableContent.excludingDesktopWindows(
                false,
                onScreenWindowsOnly: true
            )
            let requestedID = number(arguments["display_id"]).map { CGDirectDisplayID($0) }
            let requestedIndex = integer(arguments["display_index"], default: 1, range: 1...max(shareable.displays.count, 1))
            let display = requestedID.flatMap { id in
                shareable.displays.first { $0.displayID == id }
            } ?? shareable.displays.first { $0.displayID == CGMainDisplayID() }
                ?? shareable.displays.dropFirst(requestedIndex - 1).first
            guard let display else {
                return .failure(String(localized: "No capturable Mac display is available."))
            }

            let displayBounds = CGDisplayBounds(display.displayID)
            var sourceRect = displayBounds
            if let x = number(arguments["x"]),
               let y = number(arguments["y"]),
               let width = number(arguments["width"]),
               let height = number(arguments["height"]), width > 0, height > 0 {
                let requested = CGRect(x: x, y: y, width: width, height: height)
                sourceRect = displayBounds.intersection(requested)
                guard !sourceRect.isNull, !sourceRect.isEmpty else {
                    return .failure(String(localized: "The requested screenshot crop does not intersect the selected display."))
                }
            }

            let filter = SCContentFilter(display: display, excludingWindows: [])
            let configuration = SCStreamConfiguration()
            let scaleX = CGFloat(display.width) / max(displayBounds.width, 1)
            let scaleY = CGFloat(display.height) / max(displayBounds.height, 1)
            configuration.sourceRect = CGRect(
                x: sourceRect.minX - displayBounds.minX,
                y: sourceRect.minY - displayBounds.minY,
                width: sourceRect.width,
                height: sourceRect.height
            )
            configuration.width = max(Int(sourceRect.width * scaleX), 1)
            configuration.height = max(Int(sourceRect.height * scaleY), 1)
            configuration.showsCursor = bool(arguments["include_cursor"] ?? arguments["cursor"], default: true)

            let image = try await SCScreenshotManager.captureImage(
                contentFilter: filter,
                configuration: configuration
            )
            let path = string(arguments["path"])
                ?? FileManager.default.temporaryDirectory
                    .appendingPathComponent("nola-screenshot-\(UUID().uuidString).png").path
            let destination = URL(fileURLWithPath: path).standardizedFileURL
            try FileManager.default.createDirectory(
                at: destination.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            guard let data = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
                return .failure(String(localized: "Nola captured the display but could not encode the PNG image."))
            }
            try data.write(to: destination, options: .atomic)
            MacRuntimePermissionEvidence.recordScreenCaptureSuccess()
            return .ok([
                "path": destination.path,
                "display_id": Int(display.displayID),
                "width": image.width,
                "height": image.height,
                "screen_recording_permission": true,
                "permission_subject": MacSystemPermissionProbe.permissionSubject(),
            ], "Mac screenshot captured")
        } catch {
            let permission = CGPreflightScreenCaptureAccess()
            return .failure(
                permission
                    ? "Nola could not capture the Mac display: \(error.localizedDescription)"
                    : "Screen Recording permission is not active for the current Nola app. Grant Nola access in System Settings; if it was just granted, quit and reopen Nola once."
            )
        }
    }

    private static func executeAccessibility(
        toolName: String,
        arguments: [String: Any]
    ) -> Result {
        let permission = MacSystemPermissionProbe.accessibilitySnapshot()
        guard let root = applicationRoot(arguments) else {
            if !permission.isAuthorized {
                _ = MacSystemPermissionProbe.accessibility(prompt: true)
                return accessibilityPermissionFailure()
            }
            return .failure(String(localized: "No accessible application is focused or matched the requested app."))
        }
        let targetProbe = MacSystemPermissionProbe.accessibilityElementProbe(root)
        if targetProbe.isReadable {
            MacRuntimePermissionEvidence.recordAccessibilitySuccess()
        }
        switch permission.targetState(isReadable: targetProbe.isReadable) {
        case .readable:
            break
        case .permissionRequired:
            _ = MacSystemPermissionProbe.accessibility(prompt: true)
            return accessibilityPermissionFailure()
        case .targetUnavailable:
            return .failure(
                "Accessibility is authorized for Nola, but \(requestedApplicationName(arguments)) "
                    + "did not expose a readable accessibility tree. Bring the target app to the front, "
                    + "open a normal app window, and retry."
            )
        }
        let maxNodes = integer(arguments["max_nodes"], default: 500, range: 1...2_000)
        let maxDepth = integer(arguments["max_depth"], default: 8, range: 1...20)
        let nodes = snapshot(root: root, maxNodes: maxNodes, maxDepth: maxDepth)
        let applicationName = requestedApplicationName(arguments)
        let hasReadableDescendants = nodes.contains { $0.depth > 0 }

        switch toolName {
        case "computer_accessibility_snapshot", "computer_observe":
            guard hasReadableDescendants else {
                return .failure(
                    "Accessibility is authorized, but \(applicationName) exposed no readable windows or child elements. Bring the target app to the front and retry."
                )
            }
            return .ok([
                "node_count": nodes.count,
                "nodes": nodes.map(\.payload),
                "accessibility_permission": true,
                "accessibility_tree_readable": true,
                "application": applicationName,
            ], "Accessibility snapshot")
        case "computer_read_text":
            let text = nodes.compactMap { nodeText($0.payload) }.filter { !$0.isEmpty }
            return .ok(
                ["text": text.joined(separator: "\n"), "node_count": nodes.count],
                String(localized: "Accessible text read")
            )
        case "computer_find_text":
            let matches = matchingNodes(nodes, arguments: arguments)
            return .ok(
                ["found": !matches.isEmpty, "match_count": matches.count, "matches": matches.map(\.payload)],
                String(localized: "Accessible text searched")
            )
        case "computer_assert_text", "computer_assert_element":
            let matches = matchingNodes(nodes, arguments: arguments)
            let expected = bool(arguments["exists"], default: true)
            let passed = (!matches.isEmpty) == expected
            return passed
                ? .ok(
                    ["matched": true, "match_count": matches.count],
                    String(localized: "Accessibility assertion passed")
                )
                : .failure(String(localized: "The requested accessibility condition was not met."))
        case "computer_element_at":
            guard let x = number(arguments["x"]), let y = number(arguments["y"]) else {
                return .failure(String(localized: "x and y are required."))
            }
            var element: AXUIElement?
            let status = AXUIElementCopyElementAtPosition(root, Float(x), Float(y), &element)
            guard status == .success, let element else {
                return .failure(String(localized: "No accessible element exists at the requested position."))
            }
            return .ok(
                elementPayload(element, index: 1, depth: 0),
                String(localized: "Accessibility element found")
            )
        case "computer_click_text", "computer_click_element", "computer_press_element", "computer_select_text":
            return perform(arguments, nodes: nodes, action: kAXPressAction as String)
        case "computer_focus_element":
            guard let node = selectedNode(nodes, arguments: arguments) else {
                return .failure(String(localized: "No accessibility element matched the request."))
            }
            let status = AXUIElementSetAttributeValue(node.element, kAXFocusedAttribute as CFString, kCFBooleanTrue)
            return status == .success
                ? .ok(node.payload, String(localized: "Accessibility element focused"))
                : .failure(
                    String(
                        format: String(localized: "The matched element cannot receive focus (AX error %lld)."),
                        Int64(status.rawValue)
                    )
                )
        case "computer_type_element", "computer_set_element_value":
            guard let node = selectedNode(nodes, arguments: arguments),
                  let text = string(arguments["text"] ?? arguments["value"]) else {
                return .failure(String(localized: "A matching element and text/value are required."))
            }
            let status = AXUIElementSetAttributeValue(node.element, kAXValueAttribute as CFString, text as CFTypeRef)
            return status == .success
                ? .ok(node.payload, String(localized: "Accessibility value updated"))
                : .failure(
                    String(
                        format: String(localized: "The matched element does not accept a value (AX error %lld)."),
                        Int64(status.rawValue)
                    )
                )
        case "computer_perform_element_action":
            let action = string(arguments["action"]) ?? (kAXPressAction as String)
            return perform(arguments, nodes: nodes, action: action)
        case "computer_move_to_element":
            guard let node = selectedNode(nodes, arguments: arguments),
                  let frame = frame(node.element) else {
                return .failure(String(localized: "No matching accessibility element with a visible frame was found."))
            }
            return postMove(CGPoint(x: frame.midX, y: frame.midY))
        default:
            return .failure(String.localizedStringWithFormat(
                String(localized: "The Nola Mac accessibility bridge does not support %@."),
                toolName
            ))
        }
    }

    private static func accessibilityPermissionFailure() -> Result {
        let subject = MacSystemPermissionProbe.permissionSubject()
        let bundleID = subject["bundle_id"] ?? "unknown"
        let executable = subject["executable"] ?? "unknown"
        let signingIdentifier = subject["signing_identifier"] ?? "unknown"
        let teamIdentifier = subject["team_identifier"] ?? "none"
        let stableIdentity = subject["stable_signing_identity"] == "true"
        let identityGuidance = stableIdentity
            ? "Grant that exact app access in System Settings, then return to Nola and retry."
            : "This build has an unstable ad-hoc signing identity, so an enabled older Nola entry does not authorize the current binary. Run a normally signed Nola build, remove the stale Accessibility entry, and grant the current app again."
        return .failure(
            "Accessibility is not active for the current Nola process "
                + "(bundle: \(bundleID), signing identifier: \(signingIdentifier), "
                + "team: \(teamIdentifier), executable: \(executable)). "
                + identityGuidance
        )
    }

    private static func perform(
        _ arguments: [String: Any],
        nodes: [Node],
        action: String
    ) -> Result {
        guard let node = selectedNode(nodes, arguments: arguments) else {
            return .failure(String(localized: "No accessibility element matched the request."))
        }
        let status = AXUIElementPerformAction(node.element, action as CFString)
        return status == .success
            ? .ok(node.payload, String(localized: "Accessibility action completed"))
            : .failure(
                String(
                    format: String(localized: "The accessibility action failed with AX error %lld."),
                    Int64(status.rawValue)
                )
            )
    }

    private static func applicationRoot(_ arguments: [String: Any]) -> AXUIElement? {
        if let requested = string(arguments["app"]),
           let app = MacAccessibilityApplicationResolver.matchingApplication(requested) {
            return AXUIElementCreateApplication(app.processIdentifier)
        }
        guard let app = MacAccessibilityApplicationResolver.defaultTargetApplication() else { return nil }
        return AXUIElementCreateApplication(app.processIdentifier)
    }

    private static func snapshot(root: AXUIElement, maxNodes: Int, maxDepth: Int) -> [Node] {
        var queue: [(AXUIElement, Int)] = [(root, 0)]
        var result: [Node] = []
        var visited: Set<CFHashCode> = []
        while !queue.isEmpty, result.count < maxNodes {
            let (element, depth) = queue.removeFirst()
            guard visited.insert(CFHash(element)).inserted else { continue }
            result.append(Node(
                element: element,
                depth: depth,
                payload: elementPayload(element, index: result.count + 1, depth: depth)
            ))
            guard depth < maxDepth else { continue }
            let remaining = max(0, maxNodes - result.count)
            queue.append(contentsOf: descendantElements(element).prefix(remaining).map { ($0, depth + 1) })
        }
        return result
    }

    private static func descendantElements(_ element: AXUIElement) -> [AXUIElement] {
        var descendants: [AXUIElement] = []
        for attributeName in accessibilityDescendantAttributeNames {
            guard let value = attribute(element, attributeName as CFString) else { continue }
            if CFGetTypeID(value) == AXUIElementGetTypeID() {
                descendants.append(unsafeBitCast(value, to: AXUIElement.self))
                continue
            }
            if let elements = value as? [AXUIElement] {
                descendants.append(contentsOf: elements)
            }
        }
        return descendants
    }

    private static func requestedApplicationName(_ arguments: [String: Any]) -> String {
        if let requested = string(arguments["app"]), !requested.isEmpty {
            return requested
        }
        return MacAccessibilityApplicationResolver.defaultTargetApplication()?.localizedName
            ?? String(localized: "the frontmost application")
    }

    private static func matchingNodes(_ nodes: [Node], arguments: [String: Any]) -> [Node] {
        let query = string(arguments["text"])
            ?? string(arguments["title"])
            ?? string(arguments["label"])
            ?? string(arguments["name"])
            ?? string(arguments["identifier"])
            ?? ""
        let role = string(arguments["role"])?.lowercased()
        let exact = string(arguments["match_mode"]) == "exact"
        return nodes.filter { node in
            if let role,
               (node.payload["role"] as? String)?.lowercased() != role {
                return false
            }
            guard !query.isEmpty else { return true }
            let candidate = nodeText(node.payload).lowercased()
            return exact ? candidate == query.lowercased() : candidate.contains(query.lowercased())
        }
    }

    private static func selectedNode(_ nodes: [Node], arguments: [String: Any]) -> Node? {
        let matches = matchingNodes(nodes, arguments: arguments)
        let occurrence = integer(arguments["occurrence"], default: 1, range: 1...100)
        return matches.dropFirst(occurrence - 1).first
    }

    private static func elementPayload(_ element: AXUIElement, index: Int, depth: Int) -> [String: Any] {
        var payload: [String: Any] = ["index": index, "depth": depth]
        for (key, axKey) in [
            ("role", kAXRoleAttribute), ("subrole", kAXSubroleAttribute),
            ("title", kAXTitleAttribute), ("label", kAXDescriptionAttribute),
            ("value", kAXValueAttribute), ("identifier", kAXIdentifierAttribute),
            ("help", kAXHelpAttribute),
        ] {
            if let value = attribute(element, axKey as CFString),
               let string = scalarString(value), !string.isEmpty {
                payload[key] = string
            }
        }
        if let enabled = attribute(element, kAXEnabledAttribute as CFString) as? Bool { payload["enabled"] = enabled }
        if let focused = attribute(element, kAXFocusedAttribute as CFString) as? Bool { payload["focused"] = focused }
        if let frame = frame(element) {
            payload["frame"] = ["x": frame.minX, "y": frame.minY, "width": frame.width, "height": frame.height]
        }
        return payload
    }

    private static func nodeText(_ payload: [String: Any]) -> String {
        ["title", "label", "value", "identifier", "help"]
            .compactMap { payload[$0] as? String }
            .filter { !$0.isEmpty }
            .joined(separator: " | ")
    }

    private static func attribute(_ element: AXUIElement, _ name: CFString) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, name, &value) == .success else { return nil }
        return value
    }

    private static func scalarString(_ value: CFTypeRef) -> String? {
        if let value = value as? String { return value }
        if let value = value as? NSNumber { return value.stringValue }
        return nil
    }

    private static func frame(_ element: AXUIElement) -> CGRect? {
        guard let positionValue = attribute(element, kAXPositionAttribute as CFString),
              let sizeValue = attribute(element, kAXSizeAttribute as CFString),
              CFGetTypeID(positionValue) == AXValueGetTypeID(),
              CFGetTypeID(sizeValue) == AXValueGetTypeID() else { return nil }
        var point = CGPoint.zero
        var size = CGSize.zero
        guard AXValueGetValue(positionValue as! AXValue, .cgPoint, &point),
              AXValueGetValue(sizeValue as! AXValue, .cgSize, &size) else { return nil }
        return CGRect(origin: point, size: size)
    }

    private static func move(_ arguments: [String: Any]) -> Result {
        if let failure = accessibilityInputFailure() { return failure }
        guard let x = number(arguments["x"]), let y = number(arguments["y"]) else {
            return .failure(String(localized: "x and y are required."))
        }
        return postMove(CGPoint(x: x, y: y))
    }

    private static func postMove(_ point: CGPoint) -> Result {
        guard let event = CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left) else {
            return .failure(String(localized: "Unable to create a pointer event."))
        }
        event.post(tap: .cghidEventTap)
        return .ok(["x": point.x, "y": point.y], String(localized: "Pointer moved"))
    }

    private static func mouseButton(_ arguments: [String: Any], down: Bool) -> Result {
        if let failure = accessibilityInputFailure() { return failure }
        let point = CGPoint(
            x: number(arguments["x"]) ?? CGEvent(source: nil)?.location.x ?? 0,
            y: number(arguments["y"]) ?? CGEvent(source: nil)?.location.y ?? 0
        )
        let button = mouseButton(arguments)
        let type: CGEventType = button == .right
            ? (down ? .rightMouseDown : .rightMouseUp)
            : (down ? .leftMouseDown : .leftMouseUp)
        guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: button) else {
            return .failure(String(localized: "Unable to create a pointer event."))
        }
        event.post(tap: .cghidEventTap)
        return .ok(
            ["x": point.x, "y": point.y],
            down ? String(localized: "Pointer pressed") : String(localized: "Pointer released")
        )
    }

    private static func click(_ arguments: [String: Any]) -> Result {
        let down = mouseButton(arguments, down: true)
        guard !down.isError else { return down }
        return mouseButton(arguments, down: false)
    }

    private static func drag(_ arguments: [String: Any]) -> Result {
        if let failure = accessibilityInputFailure() { return failure }
        guard let fromX = number(arguments["from_x"] ?? arguments["x"]),
              let fromY = number(arguments["from_y"] ?? arguments["y"]),
              let toX = number(arguments["to_x"]),
              let toY = number(arguments["to_y"]) else {
            return .failure(String(localized: "from_x, from_y, to_x, and to_y are required."))
        }
        let button = mouseButton(arguments)
        let start = CGPoint(x: fromX, y: fromY)
        let end = CGPoint(x: toX, y: toY)
        CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: start, mouseButton: button)?.post(tap: .cghidEventTap)
        CGEvent(mouseEventSource: nil, mouseType: .leftMouseDragged, mouseCursorPosition: end, mouseButton: button)?.post(tap: .cghidEventTap)
        CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: end, mouseButton: button)?.post(tap: .cghidEventTap)
        return .ok(
            ["from": [fromX, fromY], "to": [toX, toY]],
            String(localized: "Pointer dragged")
        )
    }

    private static func scroll(_ arguments: [String: Any]) -> Result {
        if let failure = accessibilityInputFailure() { return failure }
        let vertical = Int32(number(arguments["delta_y"] ?? arguments["y"] ?? arguments["amount"]) ?? 0)
        let horizontal = Int32(number(arguments["delta_x"] ?? arguments["x"]) ?? 0)
        guard let event = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 2, wheel1: vertical, wheel2: horizontal, wheel3: 0) else {
            return .failure(String(localized: "Unable to create a scroll event."))
        }
        event.post(tap: .cghidEventTap)
        return .ok(["delta_x": horizontal, "delta_y": vertical], String(localized: "Scrolled"))
    }

    private static func mouseButton(_ arguments: [String: Any]) -> CGMouseButton {
        switch string(arguments["button"])?.lowercased() {
        case "right": return .right
        case "center", "middle": return .center
        default: return .left
        }
    }

    private static func getClipboard() -> Result {
        .ok(
            ["text": NSPasteboard.general.string(forType: .string) ?? ""],
            String(localized: "Clipboard read")
        )
    }

    private static func setClipboard(_ arguments: [String: Any]) -> Result {
        let value = string(arguments["text"]) ?? ""
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        guard pasteboard.setString(value, forType: .string) else {
            return .failure(String(localized: "Nola could not update the clipboard."))
        }
        return .ok(["character_count": value.count], String(localized: "Clipboard updated"))
    }

    private static func typeText(_ arguments: [String: Any]) async -> Result {
        if let failure = accessibilityInputFailure(prompt: true) { return failure }
        let value = stringPreservingWhitespace(arguments["text"]) ?? ""
        if bool(arguments["clear"], default: false) {
            let selection = postKeyChord(keys: ["command", "a"])
            guard !selection.isError else { return selection }
        }
        guard !value.isEmpty else {
            return .ok(["character_count": 0], String(localized: "Typed empty text"))
        }

        let pasteboard = NSPasteboard.general
        let previousItems = pasteboard.pasteboardItems?.compactMap { item -> NSPasteboardItem? in
            let copy = NSPasteboardItem()
            for type in item.types {
                if let data = item.data(forType: type) { copy.setData(data, forType: type) }
            }
            return copy
        }
        pasteboard.clearContents()
        guard pasteboard.setString(value, forType: .string) else {
            return .failure(String(localized: "Nola could not stage text on the clipboard for typing."))
        }
        let pasted = postKeyChord(keys: ["command", "v"])
        if !pasted.isError {
            try? await Task.sleep(for: .milliseconds(180))
        }
        if let previousItems {
            pasteboard.clearContents()
            pasteboard.writeObjects(previousItems)
        }
        guard !pasted.isError else { return pasted }
        if bool(arguments["press_enter"], default: false) {
            let entered = postKeyChord(keys: ["enter"])
            guard !entered.isError else { return entered }
        }
        let waitAfter = integer(arguments["wait_after_ms"], default: 250, range: 0...5_000)
        if waitAfter > 0 { try? await Task.sleep(for: .milliseconds(waitAfter)) }
        MacRuntimePermissionEvidence.recordAccessibilitySuccess()
        return .ok(
            ["character_count": value.count],
            String.localizedStringWithFormat(
                String(localized: "Typed %lld characters"),
                Int64(value.count)
            )
        )
    }

    private static func pressKey(_ arguments: [String: Any]) -> Result {
        guard let key = string(arguments["key"] ?? arguments["name"]), !key.isEmpty else {
            return .failure(String(localized: "A key is required."))
        }
        return postKeyChord(keys: [key])
    }

    private static func hotkey(_ arguments: [String: Any]) -> Result {
        guard let keys = stringArray(arguments["keys"]), !keys.isEmpty else {
            return .failure(String(localized: "A non-empty keys array is required."))
        }
        return postKeyChord(keys: keys)
    }

    private static func keyTransition(_ arguments: [String: Any], down: Bool) -> Result {
        if let failure = accessibilityInputFailure(prompt: true) { return failure }
        guard let key = string(arguments["key"] ?? arguments["name"]),
              let keyCode = MacKeyboardEventMapper.keyCode(for: key) else {
            return .failure(String(localized: "The requested key is not supported."))
        }
        guard let event = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: down) else {
            return .failure(String(localized: "Nola could not create the keyboard event."))
        }
        event.post(tap: .cghidEventTap)
        MacRuntimePermissionEvidence.recordAccessibilitySuccess()
        return .ok(
            ["key": key, "down": down],
            down ? String(localized: "Key pressed") : String(localized: "Key released")
        )
    }

    private static func copySelection(_ arguments: [String: Any]) async -> Result {
        if let failure = accessibilityInputFailure(prompt: true) { return failure }
        let pasteboard = NSPasteboard.general
        let previous = pasteboard.string(forType: .string)
        let copied = postKeyChord(keys: ["command", "c"])
        guard !copied.isError else { return copied }
        let waitAfter = integer(arguments["wait_after_ms"], default: 250, range: 0...5_000)
        if waitAfter > 0 { try? await Task.sleep(for: .milliseconds(waitAfter)) }
        let value = pasteboard.string(forType: .string) ?? ""
        if bool(arguments["restore_clipboard"], default: true), let previous {
            pasteboard.clearContents()
            pasteboard.setString(previous, forType: .string)
        }
        let maxCharacters = integer(arguments["max_chars"], default: 4_000, range: 1...50_000)
        let clipped = String(value.prefix(maxCharacters))
        MacRuntimePermissionEvidence.recordAccessibilitySuccess()
        return .ok([
            "text": clipped,
            "character_count": value.count,
            "truncated": clipped.count < value.count,
        ], "Copied selected text")
    }

    private static func postKeyChord(keys: [String]) -> Result {
        if let failure = accessibilityInputFailure(prompt: true) { return failure }
        guard let primary = keys.last,
              let keyCode = MacKeyboardEventMapper.keyCode(for: primary) else {
            return .failure(String(localized: "The requested key chord is not supported."))
        }
        let flags = MacKeyboardEventMapper.flags(for: Array(keys.dropLast()))
        guard let down = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: true),
              let up = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: false) else {
            return .failure(String(localized: "Nola could not create the keyboard shortcut."))
        }
        down.flags = flags
        up.flags = flags
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
        MacRuntimePermissionEvidence.recordAccessibilitySuccess()
        return .ok(["keys": keys], String(localized: "Keyboard shortcut sent"))
    }

    private static func accessibilityInputFailure(prompt: Bool = false) -> Result? {
        guard !MacSystemPermissionProbe.accessibilitySnapshot(prompt: prompt).isAuthorized else { return nil }
        return accessibilityPermissionFailure()
    }

    private static func string(_ value: Any?) -> String? {
        (value as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func stringPreservingWhitespace(_ value: Any?) -> String? {
        value as? String
    }

    private static func stringArray(_ value: Any?) -> [String]? {
        (value as? [Any])?.compactMap { string($0) }
    }

    private static func number(_ value: Any?) -> Double? {
        if let value = value as? NSNumber { return value.doubleValue }
        if let value = value as? String { return Double(value) }
        return nil
    }

    private static func bool(_ value: Any?, default defaultValue: Bool) -> Bool {
        (value as? NSNumber)?.boolValue ?? defaultValue
    }

    private static func integer(_ value: Any?, default defaultValue: Int, range: ClosedRange<Int>) -> Int {
        min(max((value as? NSNumber)?.intValue ?? defaultValue, range.lowerBound), range.upperBound)
    }
}

enum MacKeyboardEventMapper {
    static func keyCode(for rawKey: String) -> CGKeyCode? {
        let key = rawKey.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return keyCodes[key]
    }

    static func flags(for modifiers: [String]) -> CGEventFlags {
        modifiers.reduce(into: CGEventFlags()) { result, modifier in
            switch modifier.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
            case "command", "cmd", "meta": result.insert(.maskCommand)
            case "shift": result.insert(.maskShift)
            case "option", "alt": result.insert(.maskAlternate)
            case "control", "ctrl": result.insert(.maskControl)
            default: break
            }
        }
    }

    private static let keyCodes: [String: CGKeyCode] = [
        "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7,
        "c": 8, "v": 9, "b": 11, "q": 12, "w": 13, "e": 14, "r": 15,
        "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21, "6": 22,
        "5": 23, "=": 24, "9": 25, "7": 26, "-": 27, "8": 28, "0": 29,
        "]": 30, "o": 31, "u": 32, "[": 33, "i": 34, "p": 35,
        "enter": 36, "return": 36, "l": 37, "j": 38, "'": 39, "k": 40,
        ";": 41, "\\": 42, ",": 43, "/": 44, "n": 45, "m": 46, ".": 47,
        "tab": 48, "space": 49, "backspace": 51, "delete": 51, "escape": 53,
        "esc": 53, "command": 55, "cmd": 55, "shift": 56, "option": 58,
        "alt": 58, "control": 59, "ctrl": 59, "f1": 122, "f2": 120,
        "f3": 99, "f4": 118, "f5": 96, "f6": 97, "f7": 98, "f8": 100,
        "f9": 101, "f10": 109, "f11": 103, "f12": 111, "home": 115,
        "end": 119, "pageup": 116, "page_up": 116, "pagedown": 121,
        "page_down": 121, "left": 123, "left_arrow": 123, "right": 124,
        "right_arrow": 124, "down": 125, "down_arrow": 125, "up": 126,
        "up_arrow": 126,
    ]
}
