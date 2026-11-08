// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NotificationIconView logic in the iOS UI layer.
// - Interfaces:
//   - NotificationIconView
//   - NotificationIconView_Previews
// - APIs:
//   - none
//
import SwiftUI
import Combine

struct NotificationIconView: View {
    @EnvironmentObject var container: ServiceContainer
    @State private var showCenter: Bool = false
    @State private var unreadCount: Int = 0

    var body: some View {
        let nm = container.notificationManager
        Button(action: { showCenter = true }) {
            ZStack(alignment: .topTrailing) {
                Image(systemName: unreadCount > 0 ? "bell.fill" : "bell")
                    .font(.title2)
                    .foregroundColor(.primary)
                if unreadCount > 0 {
                    Text(unreadCount > 99 ? "99+" : String(unreadCount))
                        .font(.system(size: 9, weight: .bold, design: .rounded))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 4)
                        .frame(minWidth: 16, minHeight: 16)
                        .background(Color.red, in: Capsule())
                        .offset(x: 9, y: -8)
                        .accessibilityLabel(Text(NSLocalizedString("notifications.unread.label", value: "未读通知", comment: "Unread notifications indicator label")))
                }
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(L10n.notificationsTitle))
        .onAppear {
            self.unreadCount = nm?.unreadCount ?? 0
        }
        .onReceive({ () -> AnyPublisher<Int, Never> in
            if let nm = nm { return nm.$unreadCount.eraseToAnyPublisher() }
            return Just(0).eraseToAnyPublisher()
        }(), perform: { v in
            self.unreadCount = v
        })
        .accessibilityHint(
            Text(
                unreadCount > 0
                ? String(
                    format: NSLocalizedString("notifications.unread.hint.count", value: "你有 %d 条未读通知", comment: "Unread notifications hint with count"),
                    unreadCount
                  )
                : NSLocalizedString("notifications.unread.hint.none", value: "没有未读通知", comment: "No unread notifications hint")
            )
        )
        .sheet(isPresented: $showCenter) {
            NotificationCenterView()
                .environmentObject(container)
        }
    }
}

struct NotificationIconView_Previews: PreviewProvider {
    static var previews: some View {
        NotificationIconView()
            .environmentObject(ServiceContainer())
            .previewLayout(.sizeThatFits)
    }
}
