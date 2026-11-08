// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ScheduledTimeParser logic in the iOS service layer.
// - Interfaces:
//   - ScheduledTimeParser
// - APIs:
//   - none
//
import Foundation

enum ScheduledTimeParser {
    /// Parses a scheduled time representation into (hour, minute).
    ///
    /// Supported inputs:
    /// - "HH:mm" (preferred)
    /// - "HH:mm:ss" (seconds ignored)
    /// - ISO-8601 date-times (e.g. "2026-01-10T08:00:00+08:00")
    /// - Full-width colon variants (e.g. NSLocalizedString("autogen.ScheduledTimeParser.2d4817f7", comment: "AUTOGEN: 08：30"))
    /// - Whitespace/newline padded strings
    /// - "HHmm" (e.g. "0830")
    static func parseHourMinute(_ raw: String) -> (hour: Int, minute: Int)? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        let normalized = trimmed.replacingOccurrences(of: "：", with: ":")

        // Handle localized meridiem formats like NSLocalizedString("autogen.ScheduledTimeParser.e05453c8", comment: "AUTOGEN") / NSLocalizedString("autogen.ScheduledTimeParser.b9ccd93e", comment: "AUTOGEN").
        if let hm = parseLocalizedMeridiemHourMinute(normalized) {
            return hm
        }

        // Try ISO-8601 first if it looks like a date-time.
        if normalized.contains("T") {
            let iso = ISO8601DateFormatter()
            if let date = iso.date(from: normalized) {
                let cal = Calendar.current
                let h = cal.component(.hour, from: date)
                let m = cal.component(.minute, from: date)
                guard (0...23).contains(h), (0...59).contains(m) else { return nil }
                return (h, m)
            }
        }

        // Try HHmm compact form.
        if normalized.count == 4, normalized.allSatisfy({ $0.isNumber }) {
            let hStr = String(normalized.prefix(2))
            let mStr = String(normalized.suffix(2))
            if let h = Int(hStr), let m = Int(mStr), (0...23).contains(h), (0...59).contains(m) {
                return (h, m)
            }
        }

        // Try HH:mm or HH:mm:ss
        let parts = normalized.split(separator: ":", omittingEmptySubsequences: true)
        guard parts.count >= 2 else { return nil }

        let hStr = String(parts[0]).trimmingCharacters(in: .whitespacesAndNewlines)
        let mStr = String(parts[1]).trimmingCharacters(in: .whitespacesAndNewlines)
        guard let h = Int(hStr), let m = Int(mStr) else { return nil }
        guard (0...23).contains(h), (0...59).contains(m) else { return nil }
        return (h, m)
    }

    /// Formats a Date into canonical "HH:mm" for storage (locale-independent).
    static func formatHHmm(from date: Date, timeZone: TimeZone = .current) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.timeZone = timeZone
        formatter.dateFormat = "HH:mm"
        return formatter.string(from: date)
    }

    /// Normalizes a scheduled time string into canonical "HH:mm".
    static func normalizeToHHmm(_ raw: String) -> String? {
        guard let hm = parseHourMinute(raw) else { return nil }
        return String(format: "%02d:%02d", hm.hour, hm.minute)
    }

    /// Normalizes schedule frequency into canonical values used by the app.
    /// Returns nil if the schedule should be treated as disabled.
    static func normalizeFrequency(_ raw: String?) -> String? {
        guard let raw else { return nil }
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let lower = trimmed.lowercased()
        if lower == "none" || lower == "off" || lower == "disabled" { return nil }
        if lower == "once" || lower == "one_time" || lower == "one-time" { return "once" }
        if lower == "daily" { return "daily" }
        if lower == "weekly" { return "weekly" }
        // Unknown values are treated as disabled to avoid silent mis-scheduling.
        return nil
    }

    private static func parseLocalizedMeridiemHourMinute(_ normalized: String) -> (hour: Int, minute: Int)? {
        let s = normalized.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !s.isEmpty else { return nil }

        // Prefix forms: NSLocalizedString("autogen.ScheduledTimeParser.1e38b4e4", comment: "AUTOGEN"), NSLocalizedString("autogen.ScheduledTimeParser.ad12f310", comment: "AUTOGEN"), NSLocalizedString("autogen.ScheduledTimeParser.35be2453", comment: "AUTOGEN"), NSLocalizedString("autogen.ScheduledTimeParser.0cb01238", comment: "AUTOGEN")
        if let match = s.firstMatch(of: /^(上午|下午|中午|晚上)\s*(\d{1,2})\s*:\s*(\d{2})(?:\s*:\s*\d{2})?$/) {
            let meridiem = String(match.1)
            guard let hour = Int(match.2), let minute = Int(match.3) else { return nil }
            guard (1...12).contains(hour), (0...59).contains(minute) else { return nil }
            let h: Int
            switch meridiem {
            case NSLocalizedString("autogen.ScheduledTimeParser.e214ee22", comment: "AUTOGEN"):
                h = (hour == 12) ? 0 : hour
            case NSLocalizedString("autogen.ScheduledTimeParser.1dfaab65", comment: "AUTOGEN"), NSLocalizedString("autogen.ScheduledTimeParser.76a41594", comment: "AUTOGEN"):
                h = (hour == 12) ? 12 : (hour + 12)
            case NSLocalizedString("autogen.ScheduledTimeParser.0c5382c1", comment: "AUTOGEN"):
                // Treat NSLocalizedString("autogen.ScheduledTimeParser.637448a8", comment: "AUTOGEN") as 12:xx; for 1-11, prefer PM semantics.
                h = (hour == 12) ? 12 : (hour + 12)
            default:
                return nil
            }
            guard (0...23).contains(h) else { return nil }
            return (h, minute)
        }

        // Suffix forms: "11:31 PM" / "9:05 am"
        if let match = s.firstMatch(of: /^(\d{1,2})\s*:\s*(\d{2})\s*([AaPp][Mm])$/) {
            guard let hour = Int(match.1), let minute = Int(match.2) else { return nil }
            guard (1...12).contains(hour), (0...59).contains(minute) else { return nil }
            let ampm = String(match.3).lowercased()
            let h: Int
            if ampm == "am" {
                h = (hour == 12) ? 0 : hour
            } else {
                h = (hour == 12) ? 12 : (hour + 12)
            }
            return (h, minute)
        }

        // Prefix forms: "PM 11:31" / "am 9:05"
        if let match = s.firstMatch(of: /^([AaPp][Mm])\s*(\d{1,2})\s*:\s*(\d{2})$/) {
            guard let hour = Int(match.2), let minute = Int(match.3) else { return nil }
            guard (1...12).contains(hour), (0...59).contains(minute) else { return nil }
            let ampm = String(match.1).lowercased()
            let h: Int
            if ampm == "am" {
                h = (hour == 12) ? 0 : hour
            } else {
                h = (hour == 12) ? 12 : (hour + 12)
            }
            return (h, minute)
        }

        return nil
    }
}
