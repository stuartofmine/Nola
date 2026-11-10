// SPDX-License-Identifier: Apache-2.0
import SwiftUI

/// Converts the resolved value for native display; never evaluates bindings.
struct AppleUIDateInput: View {
    let label: String
    let rawValue: String
    let enableDate: Bool
    let enableTime: Bool
    let minimum: String?
    let maximum: String?
    let changed: (String) -> Void
    @State private var emptyDisplayDate = Date()

    private var components: DatePickerComponents {
        enableDate ? (enableTime ? [.date, .hourAndMinute] : [.date]) : [.hourAndMinute]
    }
    private var selection: Binding<Date> {
        Binding(get: { parse(rawValue) ?? Self.emptySelectionDate(now: emptyDisplayDate, minimum: minimum.flatMap(parse), maximum: maximum.flatMap(parse)) }, set: { date in
            emptyDisplayDate = date
            if enableDate && enableTime { changed(ISO8601DateFormatter().string(from: date)) }
            else { changed(formatter.string(from: date)) }
        })
    }
    /// Native pickers clamp their visual selection to the allowed range. Keep
    /// explicit acceptance of an empty field identical to that displayed date.
    nonisolated static func emptySelectionDate(now: Date, minimum: Date?, maximum: Date?) -> Date {
        if let minimum, let maximum, minimum <= maximum { return min(max(now, minimum), maximum) }
        if let minimum { return max(now, minimum) }
        if let maximum { return min(now, maximum) }
        return now
    }

    private var formatter: DateFormatter {
        let value = DateFormatter()
        value.locale = Locale(identifier: "en_US_POSIX")
        value.calendar = Calendar(identifier: .gregorian)
        value.dateFormat = enableDate ? "yyyy-MM-dd" : "HH:mm"
        value.isLenient = false
        return value
    }
    private func parse(_ value: String) -> Date? {
        let iso = ISO8601DateFormatter()
        if let date = iso.date(from: value) { return date }
        iso.formatOptions.insert(.withFractionalSeconds)
        return iso.date(from: value) ?? formatter.date(from: value)
    }
    var body: some View {
        if !enableDate && !enableTime {
            Text(rawValue.isEmpty ? label : rawValue)
        } else {
        VStack(alignment: .leading, spacing: 6) {
            if let minimum = minimum.flatMap(parse), let maximum = maximum.flatMap(parse), minimum <= maximum {
                DatePicker(label, selection: selection, in: minimum...maximum, displayedComponents: components)
            } else if let minimum = minimum.flatMap(parse) {
                DatePicker(label, selection: selection, in: minimum..., displayedComponents: components)
            } else if let maximum = maximum.flatMap(parse) {
                DatePicker(label, selection: selection, in: ...maximum, displayedComponents: components)
            } else {
                DatePicker(label, selection: selection, displayedComponents: components)
            }
            if rawValue.isEmpty {
                // DatePicker needs a display date; do not silently commit today.
                Button("使用所示日期或时间") {
                    let displayed = selection.wrappedValue
                    selection.wrappedValue = displayed
                }
            } else {
                Button("清除") { changed("") }.buttonStyle(.borderless)
            }
        }
        }
    }
}
