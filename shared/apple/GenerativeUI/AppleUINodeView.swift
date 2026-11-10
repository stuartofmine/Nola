// SPDX-License-Identifier: Apache-2.0
// Shared iOS/macOS renderer for Core-resolved component instances.
import SwiftUI

/// Host owns draft lifetime and presentation; this view only draws resolved nodes.
/// The node ID, rather than its revision, is the SwiftUI identity so edits do not
/// recreate focused controls during a streamed update.
struct AppleUINodeView<JSON: JournalJSONValue>: View {
    let id: String
    let snapshot: AppleUISnapshot<JSON>
    let value: (String, String, JSON?) -> JSON?
    let edit: (String, String, JSON) -> Void
    let dispatch: (AppleUIActionReference) -> Void
    let present: (String) -> Void
    var depth = 0
    var presentationOnly = false
    var actionsDisabled = false
    @State private var selectedTab = 0
    @FocusState private var textFocused: Bool

    var body: some View {
        if depth <= 32, let node = snapshot.nodes[id] {
            VStack(alignment: .leading, spacing: 6) {
                component(node)
                ForEach(Array(node.validation_errors.enumerated()), id: \.offset) { _, message in
                    Label(message, systemImage: "exclamationmark.circle")
                        .font(.caption).foregroundStyle(.red)
                }
            }
            .disabled(snapshot.phase != .ready || scalar(node.props["disabled"], as: Bool.self) == true)
        }
    }

    @ViewBuilder private func component(_ node: AppleUINode<JSON>) -> some View {
        if presentationOnly && ["TextField", "CheckBox", "Slider", "ChoicePicker", "DateTimeInput"].contains(node.component) {
            // A Button label is presentation, not a disabled nested control.
            // Disabled label descendants incorrectly mark the native AXButton disabled.
            Text(label(node))
        } else {
        switch node.component {
        case "Text":
            Text(node.props["text"]?.journalString ?? "")
                .font(textFont(node.props["variant"]?.journalString))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        case "Icon":
            AppleUIIconView(name: node.props["name"]?.journalString,
                svgPath: node.props["name"]?.journalObject?["svgPath"]?.journalString,
                label: node.props["ariaLabel"]?.journalString ?? node.props["name"]?.journalString ?? "Icon")
        case "Row", "Column", "List":
            AppleUIAxisLayout(horizontal: node.component == "Row" || (node.component == "List" && node.props["direction"]?.journalString == "horizontal"),
                justify: node.props["justify"]?.journalString ?? "start",
                align: node.props["align"]?.journalString ?? "stretch") {
                children(node, "children", weighted: node.component != "List")
            }
        case "Card":
            children(node, "child").padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color.secondary.opacity(0.25), lineWidth: 1))
        case "Divider": Divider()
        case "Button":
            if presentationOnly {
                children(node, "child", presentationOnly: true)
            } else {
            Button {
                if let action = node.actions["action"] { dispatch(action) }
            } label: {
                children(node, "child", presentationOnly: true)
                    .allowsHitTesting(false)
                    .frame(maxWidth: .infinity, minHeight: controlHeight)
                    .contentShape(Rectangle())
            }
            .modifier(AppleUIButtonVariant(variant: node.props["variant"]?.journalString ?? "default"))
            .accessibilityIdentifier("a2ui.action.\(node.component_id)")
            .disabled(actionsDisabled || node.actions["action"] == nil || !node.validation_errors.isEmpty)
            }
        case "TextField":
            VStack(alignment: .leading, spacing: 6) {
                Text(label(node))
                    .contentShape(Rectangle())
                    .onTapGesture { textFocused = true }
                if node.props["variant"]?.journalString == "obscured" {
                    SecureField(label(node), text: binding(node, default: "")).focused($textFocused)
                        .accessibilityIdentifier("a2ui.field.\(node.component_id)")
                } else if node.props["variant"]?.journalString == "number" {
                    TextField(label(node), value: binding(node, default: ""), formatter: AppleUINumberFormatter())
                        .modifier(AppleUINumericInput(enabled: true))
                        .focused($textFocused)
                        .accessibilityIdentifier("a2ui.field.\(node.component_id)")
                } else {
                    TextField(label(node), text: binding(node, default: ""), axis: node.props["variant"]?.journalString == "longText" ? .vertical : .horizontal)
                        .modifier(AppleUINumericInput(enabled: node.props["variant"]?.journalString == "number"))
                        .focused($textFocused)
                        .accessibilityIdentifier("a2ui.field.\(node.component_id)")
                }
            }.textFieldStyle(.roundedBorder).frame(minHeight: controlHeight)
                .disabled(node.bindings["value"] == nil)
        case "CheckBox":
            Toggle(label(node), isOn: binding(node, default: false))
                .frame(minHeight: controlHeight).contentShape(Rectangle())
                .disabled(node.bindings["value"] == nil)
        case "Slider":
            let minimum = scalar(node.props["min"], as: Double.self) ?? 0
            let maximum = scalar(node.props["max"], as: Double.self) ?? 100
            if minimum < maximum {
                let steps = scalar(node.props["steps"], as: Double.self) ?? 0
                VStack(alignment: .leading) {
                    Text(label(node))
                    if steps > 0 {
                        Slider(value: binding(node, default: minimum), in: minimum...maximum, step: (maximum - minimum) / steps)
                    } else {
                        Slider(value: binding(node, default: minimum), in: minimum...maximum)
                    }
                }.accessibilityLabel(label(node)).frame(minHeight: controlHeight)
                    .disabled(node.bindings["value"] == nil)
            }
        case "ChoicePicker":
            VStack(alignment: .leading, spacing: 8) {
                Text(label(node))
                ForEach(Array((node.props["options"]?.journalArray ?? []).enumerated()), id: \.offset) { _, option in
                    if let fields = option.journalObject, let optionValue = fields["value"] {
                        let selected = value(id, "value", node.props["value"])?.journalArray ?? []
                        Button {
                            var next = node.props["variant"]?.journalString == "multipleSelection" ? selected : []
                            if let index = next.firstIndex(of: optionValue) { next.remove(at: index) }
                            else { next.append(optionValue) }
                            if let encoded = encode(next) { edit(id, "value", encoded) }
                        } label: {
                            HStack {
                                Text(fields["label"]?.journalString ?? "")
                                Spacer()
                                Image(systemName: selected.contains(optionValue) ? "checkmark.circle.fill" : "circle")
                            }.frame(minHeight: controlHeight).contentShape(Rectangle())
                        }.buttonStyle(.plain)
                            .accessibilityAddTraits(selected.contains(optionValue) ? .isSelected : [])
                            .disabled(node.bindings["value"] == nil)
                    }
                }
            }
        case "DateTimeInput":
            AppleUIDateInput(label: label(node),
                rawValue: value(id, "value", node.props["value"])?.journalString ?? "",
                enableDate: scalar(node.props["enableDate"], as: Bool.self) ?? false,
                enableTime: scalar(node.props["enableTime"], as: Bool.self) ?? false,
                minimum: node.props["min"]?.journalString, maximum: node.props["max"]?.journalString,
                changed: { newValue in if let encoded = encode(newValue) { edit(id, "value", encoded) } })
                .frame(minHeight: controlHeight).disabled(node.bindings["value"] == nil)
        case "Tabs":
            let titles = node.props["tab_titles"]?.journalArray?.compactMap(\.journalString) ?? []
            VStack(alignment: .leading, spacing: 12) {
                Picker("", selection: $selectedTab) {
                    ForEach(Array(titles.enumerated()), id: \.offset) { index, title in Text(title).tag(index) }
                }.pickerStyle(.menu)
                children(node, "tab:\(titles.indices.contains(selectedTab) ? selectedTab : 0)")
            }
        case "Modal":
            if presentationOnly {
                children(node, "trigger", presentationOnly: true)
            } else {
                Button { present(id) } label: {
                    children(node, "trigger", presentationOnly: true).allowsHitTesting(false)
                        .frame(minHeight: controlHeight).contentShape(Rectangle())
                }.buttonStyle(.plain)
            }
        default:
            Text("此组件暂不可显示：\(node.component)").foregroundStyle(.secondary)
        }
        }
    }

    private func children(_ node: AppleUINode<JSON>, _ slot: String, presentationOnly labelOnly: Bool = false, weighted: Bool = false) -> some View {
        ForEach(node.slots[slot] ?? [], id: \.self) { child in
            // Type erasure breaks the recursive opaque View type, not identity.
            AnyView(AppleUINodeView(id: child, snapshot: snapshot, value: value,
                edit: edit, dispatch: dispatch, present: present, depth: depth + 1, presentationOnly: presentationOnly || labelOnly, actionsDisabled: actionsDisabled))
                .layoutValue(key: AppleUIChildWeight.self, value: weighted ? CGFloat(scalar(snapshot.nodes[child]?.props["weight"], as: Double.self) ?? 0) : 0)
        }
    }
    private func binding<T: Codable>(_ node: AppleUINode<JSON>, default fallback: T) -> Binding<T> {
        Binding(get: { scalar(value(id, "value", node.props["value"]), as: T.self) ?? fallback },
            set: { newValue in if let encoded = encode(newValue) { edit(id, "value", encoded) } })
    }
    private func encode<T: Encodable>(_ value: T) -> JSON? {
        guard let data = try? JSONEncoder().encode(value) else { return nil }
        return try? JSONDecoder().decode(JSON.self, from: data)
    }
    private func scalar<T: Decodable>(_ value: JSON?, as type: T.Type) -> T? {
        guard let value, let data = try? JSONEncoder().encode(value) else { return nil }
        return try? JSONDecoder().decode(type, from: data)
    }
    private func label(_ node: AppleUINode<JSON>) -> String {
        node.props["label"]?.journalString ?? node.props["ariaLabel"]?.journalString ?? ""
    }
    private func textFont(_ variant: String?) -> Font {
        switch variant {
        case "h1": .title
        case "h2": .title2
        case "h3": .title3
        case "h4", "h5", "h6": .headline
        case "caption": .caption
        default: .body
        }
    }
    private var controlHeight: CGFloat {
        #if os(iOS)
        44
        #else
        28
        #endif
    }
}

private struct AppleUIButtonVariant: ViewModifier {
    let variant: String
    @ViewBuilder func body(content: Content) -> some View {
        switch variant {
        case "primary": content.buttonStyle(.borderedProminent)
            .tint(Color("AccentColor"))
            .foregroundStyle(Color("NolaOnAccent"))
        case "borderless": content.buttonStyle(.borderless)
        default: content.buttonStyle(.bordered)
        }
    }
}

private struct AppleUINumericInput: ViewModifier {
    let enabled: Bool
    @ViewBuilder func body(content: Content) -> some View {
        #if os(iOS)
        content.keyboardType(enabled ? .numbersAndPunctuation : .default)
        #else
        // The protocol value remains a string. Do not rewrite composed text or
        // parse it into a number before Core validates the committed edit.
        content
        #endif
    }
}

/// Keeps the wire value a string, accepting incomplete native editing buffers.
/// Validation occurs on commit; marked IME text is never rewritten by a binding.
final class AppleUINumberFormatter: Formatter {
    override func string(for obj: Any?) -> String? { obj as? String }
    override func getObjectValue(_ obj: AutoreleasingUnsafeMutablePointer<AnyObject?>?, for string: String,
                                errorDescription error: AutoreleasingUnsafeMutablePointer<NSString?>?) -> Bool {
        let trimmed = string.trimmingCharacters(in: .whitespacesAndNewlines)
        let scanner = Scanner(string: trimmed)
        scanner.locale = Locale.current
        let parsed = scanner.scanDecimal()
        guard trimmed.isEmpty || (parsed != nil && scanner.isAtEnd) else {
            error?.pointee = "Enter a number"
            return false
        }
        obj?.pointee = string as NSString
        return true
    }
    override func isPartialStringValid(_ partialString: String,
                                      newEditingString: AutoreleasingUnsafeMutablePointer<NSString?>?,
                                      errorDescription: AutoreleasingUnsafeMutablePointer<NSString?>?) -> Bool { true }
}
