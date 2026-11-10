import Foundation

/// ASCII delimiters are scanned once across append boundaries. Offsets refer
/// to UTF-8 in the current pending block, never to grapheme counts or a journal.
nonisolated struct MacMarkdownTailBoundaryScanner {
    enum Kind { case table, fence(marker: UInt8, minimumCount: Int) }
    let kind: Kind
    private(set) var offset: Int
    private(set) var scannedByteCount = 0
    private(set) var boundary: Range<Int>?
    private var previousWasLF = false
    private var lineStart: Int
    private var indentation = 0
    private var markers = 0
    private var phase = 0 // indentation, markers, trailing space, CR, invalid

    init(kind: Kind, startingAt offset: Int = 0) {
        self.kind = kind
        self.offset = offset
        lineStart = offset
    }

    mutating func append<S: Sequence>(_ bytes: S) where S.Element == UInt8 {
        for byte in bytes {
            guard boundary == nil else { return }
            scannedByteCount += 1
            switch kind {
            case .table:
                if byte == 10 && previousWasLF { boundary = (offset - 1)..<(offset + 1) }
                previousWasLF = byte == 10
            case let .fence(marker, minimumCount):
                if byte == 10 {
                    if markers >= minimumCount && (1...3).contains(phase) {
                        boundary = lineStart..<(offset + 1)
                    }
                    lineStart = offset + 1
                    indentation = 0
                    markers = 0
                    phase = 0
                } else {
                    switch phase {
                    case 0:
                        if byte == 32 || byte == 9 {
                            indentation += 1
                            if indentation > 3 { phase = 4 }
                        } else if byte == marker { markers = 1; phase = 1 }
                        else { phase = 4 }
                    case 1:
                        if byte == marker { markers += 1 }
                        else if byte == 32 || byte == 9 { phase = 2 }
                        else if byte == 13 { phase = 3 }
                        else { phase = 4 }
                    case 2:
                        if byte == 13 { phase = 3 }
                        else if byte != 32 && byte != 9 { phase = 4 }
                    default: phase = 4
                    }
                }
            }
            offset += 1
        }
    }

    func closingBoundary(allowEndOfInput: Bool) -> Range<Int>? {
        if let boundary { return boundary }
        if case let .fence(_, minimumCount) = kind,
           allowEndOfInput, markers >= minimumCount, phase == 1 || phase == 2 {
            return lineStart..<offset
        }
        return nil
    }

    var partialFenceLineStart: Int? {
        guard case let .fence(_, minimumCount) = kind,
              markers > 0, markers < minimumCount, phase == 1 || phase == 2 else { return nil }
        return lineStart
    }
}
