// SPDX-License-Identifier: Apache-2.0
// Native layout of Core-resolved children; no protocol or data evaluation.
import SwiftUI

struct AppleUIChildWeight: LayoutValueKey {
    static let defaultValue: CGFloat = 0
}

struct AppleUIAxisLayout: Layout {
    let horizontal: Bool
    let justify: String
    let align: String
    var spacing: CGFloat = 12

    private func main(_ size: CGSize) -> CGFloat { horizontal ? size.width : size.height }
    private func cross(_ size: CGSize) -> CGFloat { horizontal ? size.height : size.width }
    private func size(_ main: CGFloat, _ cross: CGFloat) -> CGSize {
        horizontal ? CGSize(width: main, height: cross) : CGSize(width: cross, height: main)
    }
    private func measured(_ proposal: ProposedViewSize, _ subviews: Subviews) -> [CGSize] {
        let rawCross = horizontal ? proposal.height : proposal.width
        let availableCross = rawCross?.isFinite == true ? max(0, rawCross!) : nil
        let initial = subviews.map { view in
            view.sizeThatFits(horizontal ? ProposedViewSize(width: nil, height: availableCross) : ProposedViewSize(width: availableCross, height: nil))
        }
        let offered = horizontal ? proposal.width : proposal.height
        let natural = initial.reduce(0) { $0 + main($1) } + spacing * CGFloat(max(0, subviews.count - 1))
        let availableMain = offered?.isFinite == true ? max(0, offered!) : natural
        let surplus = max(0, availableMain - natural)
        let naturalChildren = initial.reduce(0) { $0 + main($1) }
        let availableChildren = max(0, availableMain - spacing * CGFloat(max(0, subviews.count - 1)))
        let shrink = naturalChildren > 0 ? min(1, availableChildren / naturalChildren) : 1
        let weights = subviews.map { view in
            let raw = view[AppleUIChildWeight.self]
            return raw.isFinite ? max(0, raw) : 0
        }
        let maximumWeight = weights.max() ?? 0
        let normalized = weights.map { maximumWeight > 0 ? $0 / maximumWeight : 0 }
        let totalWeight = normalized.reduce(0, +)
        return subviews.enumerated().map { index, view in
            let weight = normalized[index]
            let extra = totalWeight > 0 ? surplus * (weight / totalWeight) : (justify == "stretch" && !subviews.isEmpty ? surplus / CGFloat(subviews.count) : 0)
            let proposedMain = main(initial[index]) * shrink + extra
            let fitted = view.sizeThatFits(ProposedViewSize(size(proposedMain, availableCross ?? cross(initial[index]))))
            return size(proposedMain, cross(fitted))
        }
    }
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let measured = measured(proposal, subviews)
        let naturalMain = measured.reduce(0) { $0 + main($1) } + spacing * CGFloat(max(0, measured.count - 1))
        let naturalCross = measured.map(cross).max() ?? 0
        let offeredMain = horizontal ? proposal.width : proposal.height
        let offeredCross = horizontal ? proposal.height : proposal.width
        return size(offeredMain?.isFinite == true ? max(0, offeredMain!) : naturalMain,
                    offeredCross?.isFinite == true ? max(0, offeredCross!) : naturalCross)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let measured = measured(ProposedViewSize(bounds.size), subviews)
        let count = measured.count
        guard count > 0 else { return }
        let occupied = measured.reduce(0) { $0 + main($1) } + spacing * CGFloat(max(0, count - 1))
        let remaining = max(0, main(bounds.size) - occupied)
        var gap = spacing
        var offset: CGFloat = 0
        switch justify {
        case "end": offset = remaining
        case "center": offset = remaining / 2
        case "spaceBetween": if count > 1 { gap += remaining / CGFloat(count - 1) }
        case "spaceAround": gap += remaining / CGFloat(count); offset = remaining / CGFloat(count) / 2
        case "spaceEvenly": gap += remaining / CGFloat(count + 1); offset = remaining / CGFloat(count + 1)
        default: break
        }
        for (index, view) in subviews.enumerated() {
            let child = measured[index]
            let spareCross = max(0, cross(bounds.size) - cross(child))
            let crossOffset = align == "end" ? spareCross : (align == "center" ? spareCross / 2 : 0)
            let target = size(main(child), align == "stretch" ? cross(bounds.size) : cross(child))
            let point = horizontal ? CGPoint(x: bounds.minX + offset, y: bounds.minY + crossOffset) : CGPoint(x: bounds.minX + crossOffset, y: bounds.minY + offset)
            view.place(at: point, anchor: .topLeading, proposal: ProposedViewSize(target))
            offset += main(child) + gap
        }
    }
}
