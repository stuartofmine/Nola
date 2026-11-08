// SPDX-License-Identifier: Apache-2.0
// Native rendering of the v0.9.1 catalog's named and SVG-path icons.
import SwiftUI
import Foundation

struct AppleUIIconView: View {
    let name: String?
    let svgPath: String?
    let label: String
    var body: some View {
        let symbol = name.flatMap { AppleUIIconSymbols.names[$0] }
        let custom = svgPath.flatMap { AppleUIIconPathCache.path(for: $0) }
        return Group {
            if let symbol {
                Image(systemName: symbol).resizable().scaledToFit()
                    .rotationEffect(name == "moreVert" ? .degrees(90) : .zero)
            } else if let custom {
                custom.fill().frame(width: 24, height: 24)
            } else {
                Image(systemName: "exclamationmark.triangle")
                    .accessibilityLabel("Icon could not be rendered")
            }
        }
        .frame(width: 24, height: 24)
        .clipped()
        .accessibilityElement(children: .ignore)
        .accessibilityAddTraits(.isImage)
        .accessibilityLabel(symbol != nil || custom != nil ? label : "\(label), icon could not be rendered")
    }
}

/// Geometry cache only: no Surface identity, input state or actions are retained.
@MainActor
private enum AppleUIIconPathCache {
    private final class Entry {
        let path: Path?
        init(_ path: Path?) { self.path = path }
    }
    private static let cache: NSCache<NSString, Entry> = {
        let value = NSCache<NSString, Entry>()
        value.countLimit = 64
        value.totalCostLimit = 4 * 1024 * 1024
        return value
    }()
    static func path(for source: String) -> Path? {
        let key = source as NSString
        if let entry = cache.object(forKey: key) { return entry.path }
        let entry = Entry(try? AppleUISVGPath.parse(source))
        cache.setObject(entry, forKey: key, cost: min(source.utf8.count, 1_048_576) * 8)
        return entry.path
    }
}

nonisolated enum AppleUIIconSymbols {
    static let names: [String: String] = [
        "accountCircle": "person.crop.circle", "add": "plus", "arrowBack": "arrow.left", "arrowForward": "arrow.right",
        "attachFile": "paperclip", "calendarToday": "calendar", "call": "phone.fill", "camera": "camera.fill",
        "check": "checkmark", "close": "xmark", "delete": "trash", "download": "arrow.down.to.line",
        "edit": "pencil", "event": "calendar.badge.clock", "error": "exclamationmark.circle.fill", "fastForward": "forward.fill",
        "favorite": "heart.fill", "favoriteOff": "heart", "folder": "folder.fill", "help": "questionmark.circle",
        "home": "house.fill", "info": "info.circle", "locationOn": "mappin.circle.fill", "lock": "lock.fill",
        "lockOpen": "lock.open.fill", "mail": "envelope.fill", "menu": "line.3.horizontal", "moreVert": "ellipsis",
        "moreHoriz": "ellipsis", "notificationsOff": "bell.slash.fill", "notifications": "bell.fill", "pause": "pause.fill",
        "payment": "creditcard", "person": "person.fill", "phone": "phone", "photo": "photo", "play": "play.fill",
        "print": "printer", "refresh": "arrow.clockwise", "rewind": "backward.fill", "search": "magnifyingglass",
        "send": "paperplane.fill", "settings": "gearshape", "share": "square.and.arrow.up", "shoppingCart": "cart.fill",
        "skipNext": "forward.end.fill", "skipPrevious": "backward.end.fill", "star": "star.fill", "starHalf": "star.leadinghalf.filled",
        "starOff": "star", "stop": "stop.fill", "upload": "arrow.up.to.line", "visibility": "eye", "visibilityOff": "eye.slash",
        "volumeDown": "speaker.wave.1.fill", "volumeMute": "speaker.fill", "volumeOff": "speaker.slash.fill", "volumeUp": "speaker.wave.3.fill",
        "warning": "exclamationmark.triangle.fill",
    ]
}

/// SVG geometry only; binding, component validation and state remain in Core.
/// Arc conversion follows https://www.w3.org/TR/SVG/implnote.html#ArcImplementationNotes.
nonisolated struct AppleUISVGPath {
    enum Failure: Error { case invalid, budget }
    private var bytes: [UInt8]
    private var index = 0
    private var path = Path()
    private var current = CGPoint.zero
    private var start = CGPoint.zero
    private var cubic: CGPoint?
    private var quadratic: CGPoint?
    private var started = false

    static func parse(_ source: String) throws -> Path {
        guard source.utf8.count <= 1_048_576 else { throw Failure.budget }
        var parser = Self(bytes: Array(source.utf8))
        return try parser.read()
    }

    private mutating func separators() {
        while index < bytes.count, [UInt8(9), 10, 13, 32, 44].contains(bytes[index]) { index += 1 }
    }
    private mutating func number() throws -> Double {
        separators()
        let begin = index
        if index < bytes.count, bytes[index] == 43 || bytes[index] == 45 { index += 1 }
        var digits = 0
        while index < bytes.count, (48...57).contains(bytes[index]) { index += 1; digits += 1 }
        if index < bytes.count, bytes[index] == 46 {
            index += 1
            while index < bytes.count, (48...57).contains(bytes[index]) { index += 1; digits += 1 }
        }
        guard digits > 0 else { throw Failure.invalid }
        if index < bytes.count, bytes[index] == 101 || bytes[index] == 69 {
            index += 1
            if index < bytes.count, bytes[index] == 43 || bytes[index] == 45 { index += 1 }
            let exponent = index
            while index < bytes.count, (48...57).contains(bytes[index]) { index += 1 }
            guard index > exponent else { throw Failure.invalid }
        }
        guard let value = Double(String(decoding: bytes[begin..<index], as: UTF8.self)), value.isFinite else { throw Failure.invalid }
        return value
    }
    private mutating func flag() throws -> Bool {
        separators()
        guard index < bytes.count, bytes[index] == 48 || bytes[index] == 49 else { throw Failure.invalid }
        defer { index += 1 }
        return bytes[index] == 49
    }
    private mutating func point(relative: Bool) throws -> CGPoint {
        let x = try number(), y = try number()
        let result = CGPoint(x: x + (relative ? current.x : 0), y: y + (relative ? current.y : 0))
        guard result.x.isFinite, result.y.isFinite else { throw Failure.invalid }
        return result
    }
    private mutating func read() throws -> Path {
        var command: UInt8?
        var segments = 0
        while true {
            separators()
            guard index < bytes.count else { break }
            let next = bytes[index]
            if (65...90).contains(next) || (97...122).contains(next) { command = next; index += 1 }
            guard let selected = command else { throw Failure.invalid }
            let upper = selected >= 97 ? selected - 32 : selected
            let relative = selected >= 97
            guard started || upper == 77 else { throw Failure.invalid }
            segments += 1
            guard segments <= 262_144 else { throw Failure.budget }
            var nextCubic: CGPoint?, nextQuadratic: CGPoint?
            switch upper {
            case 77:
                current = try point(relative: relative); start = current; path.move(to: current); started = true
                command = relative ? 108 : 76
            case 76:
                current = try point(relative: relative); path.addLine(to: current)
            case 72:
                current.x = try number() + (relative ? current.x : 0)
                guard current.x.isFinite else { throw Failure.invalid }
                path.addLine(to: current)
            case 86:
                current.y = try number() + (relative ? current.y : 0)
                guard current.y.isFinite else { throw Failure.invalid }
                path.addLine(to: current)
            case 67:
                let a = try point(relative: relative), b = try point(relative: relative), end = try point(relative: relative)
                path.addCurve(to: end, control1: a, control2: b); nextCubic = b; current = end
            case 83:
                let a = cubic.map { CGPoint(x: 2 * current.x - $0.x, y: 2 * current.y - $0.y) } ?? current
                guard a.x.isFinite, a.y.isFinite else { throw Failure.invalid }
                let b = try point(relative: relative), end = try point(relative: relative)
                path.addCurve(to: end, control1: a, control2: b); nextCubic = b; current = end
            case 81:
                let a = try point(relative: relative), end = try point(relative: relative)
                path.addQuadCurve(to: end, control: a); nextQuadratic = a; current = end
            case 84:
                let a = quadratic.map { CGPoint(x: 2 * current.x - $0.x, y: 2 * current.y - $0.y) } ?? current
                guard a.x.isFinite, a.y.isFinite else { throw Failure.invalid }
                let end = try point(relative: relative)
                path.addQuadCurve(to: end, control: a); nextQuadratic = a; current = end
            case 65:
                let rx = try number(), ry = try number(), rotation = try number()
                let large = try flag(), sweep = try flag(), end = try point(relative: relative)
                try arc(to: end, rx: abs(rx), ry: abs(ry), rotation: rotation, large: large, sweep: sweep)
                current = end
            case 90:
                path.closeSubpath(); current = start; command = nil
            default: throw Failure.invalid
            }
            cubic = nextCubic; quadratic = nextQuadratic
        }
        guard started else { throw Failure.invalid }
        return path
    }

    private mutating func arc(to end: CGPoint, rx radiusX: Double, ry radiusY: Double, rotation: Double, large: Bool, sweep: Bool) throws {
        guard current != end else { return }
        guard radiusX > 0, radiusY > 0 else { path.addLine(to: end); return }
        var rx = radiusX, ry = radiusY
        let phi = rotation.truncatingRemainder(dividingBy: 360) * .pi / 180
        let c = cos(phi), s = sin(phi)
        let dx = (current.x - end.x) / 2, dy = (current.y - end.y) / 2
        let x = c * dx + s * dy, y = -s * dx + c * dy
        let scale = x * x / (rx * rx) + y * y / (ry * ry)
        guard scale.isFinite else { throw Failure.invalid }
        if scale > 1 { rx *= sqrt(scale); ry *= sqrt(scale) }
        let denominator = rx * rx * y * y + ry * ry * x * x
        guard denominator.isFinite, denominator > 0 else { throw Failure.invalid }
        let factor = (large == sweep ? -1.0 : 1.0) * sqrt(max(0, (rx * rx * ry * ry - denominator) / denominator))
        guard factor.isFinite else { throw Failure.invalid }
        let cx1 = factor * rx * y / ry, cy1 = -factor * ry * x / rx
        let cx = c * cx1 - s * cy1 + (current.x + end.x) / 2
        let cy = s * cx1 + c * cy1 + (current.y + end.y) / 2
        let ux = (x - cx1) / rx, uy = (y - cy1) / ry
        let vx = (-x - cx1) / rx, vy = (-y - cy1) / ry
        let theta = atan2(uy, ux)
        var delta = atan2(ux * vy - uy * vx, ux * vx + uy * vy)
        if !sweep && delta > 0 { delta -= 2 * .pi }
        if sweep && delta < 0 { delta += 2 * .pi }
        path.addRelativeArc(center: .zero, radius: 1, startAngle: .radians(theta), delta: .radians(delta),
            transform: CGAffineTransform(a: rx * c, b: rx * s, c: -ry * s, d: ry * c, tx: cx, ty: cy))
    }
}
