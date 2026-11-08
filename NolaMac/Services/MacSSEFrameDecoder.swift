import Foundation

/// Incremental SSE framing, independent of JSON and transport chunk boundaries.
/// Keeps empty lines (event boundaries), which AsyncBytes.lines does not promise
/// to preserve. EOF dispatches an unterminated final frame for Host compatibility.
nonisolated struct MacSSEFrameDecoder {
    private var line: [UInt8] = []
    private var dataLines: [String] = []
    private var isFirstLine = true
    private var skipLF = false
    private var isFinished = false

    mutating func append(_ byte: UInt8) -> String? {
        guard !isFinished else { return nil }
        if skipLF {
            skipLF = false
            if byte == 0x0A { return nil }
        }
        if byte == 0x0D {
            skipLF = true
            return consumeLine()
        }
        if byte == 0x0A { return consumeLine() }
        line.append(byte)
        return nil
    }

    mutating func finish() -> String? {
        guard !isFinished else { return nil }
        isFinished = true
        if !line.isEmpty, let frame = consumeLine() { return frame }
        return dispatch()
    }

    private mutating func consumeLine() -> String? {
        defer { line.removeAll(keepingCapacity: true) }
        if isFirstLine {
            isFirstLine = false
            if line.starts(with: [0xEF, 0xBB, 0xBF]) { line.removeFirst(3) }
        }
        guard !line.isEmpty else { return dispatch() }
        guard line.first != 0x3A else { return nil } // comment
        let separator = line.firstIndex(of: 0x3A) ?? line.endIndex
        guard line[..<separator].elementsEqual([0x64, 0x61, 0x74, 0x61]) else {
            return nil
        }
        var valueStart = separator == line.endIndex ? separator : separator + 1
        if valueStart < line.endIndex, line[valueStart] == 0x20 { valueStart += 1 }
        dataLines.append(String(decoding: line[valueStart...], as: UTF8.self))
        return nil
    }

    private mutating func dispatch() -> String? {
        guard !dataLines.isEmpty else { return nil }
        let frame = dataLines.joined(separator: "\n")
        dataLines.removeAll(keepingCapacity: true)
        return frame
    }
}
