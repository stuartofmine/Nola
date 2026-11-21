// SPDX-License-Identifier: Apache-2.0
import SwiftUI

struct MacRuntimePluginManagerView: View {
    private typealias Profile = MacDesktopPluginLibrary.Profile
    private typealias Package = MacDesktopPluginLibrary.Package
    let library: MacDesktopPluginLibrary?
    let libraryError: String?
    let openPage: (MacDesktopPluginPage) -> Void
    let openPairedHost: () -> Void
    let openingPairedHost: Bool

    private struct Removal: Identifiable {
        let profile: Profile
        let package: Package
        var id: String { profile.id + ":" + package.id }
    }
    @Environment(\.desktopPluginClient) private var client
    @State private var adding = false
    @State private var source = ""
    @State private var destination: String?
    @State private var removal: Removal?
    @State private var updating = false
    @State private var failure: String?
    private var profiles: [Profile] { library?.profiles ?? [] }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(spacing: 8) {
                Text("Installed plugins").font(.title2.weight(.semibold))
                Spacer()
                Button {
                    failure = nil
                    adding = true
                } label: { Label(String(localized: "Add plugin"), systemImage: "plus") }
                .buttonStyle(MacActionButtonStyle())
                .disabled(updating || library == nil)
                .accessibilityIdentifier("workspace.plugins.add")
                Menu {
                    Button(String(localized: "Refresh")) { refresh() }
                    Button(String(localized: "Manage on paired Host"), action: openPairedHost).disabled(openingPairedHost)
                } label: {
                    Image(systemName: "ellipsis").frame(width: 26, height: 26)
                }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                .accessibilityLabel(String(localized: "More"))
            }
            if let failure, !adding { errorMessage(failure) }
            if let libraryError { errorMessage(libraryError) }
            if let library {
                if library.profiles.allSatisfy({ $0.packages.isEmpty && $0.error == nil }) {
                    Label(String(localized: "No plugins installed"), systemImage: "puzzlepiece.extension")
                        .foregroundStyle(.secondary).padding(.vertical, 14)
                }
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(profiles) { profile in
                        ForEach(profile.packages) { package in
                            packageRow(package, in: profile)
                            Divider()
                        }
                        if let error = profile.error {
                            errorMessage(profile.title + " · " + error).padding(.vertical, 8)
                        }
                    }
                }
            } else if libraryError == nil {
                ProgressView().controlSize(.small)
            }
            if updating && !adding { ProgressView().controlSize(.small) }
        }
        .sheet(isPresented: $adding) {
            MacAddPluginSheet(profiles: profiles, source: $source, destination: $destination, updating: updating, failure: failure ?? libraryError, refresh: refresh,
                cancel: { adding = false; failure = nil },
                install: { mutate($0, action: "install", source: $1) })
        }
        .alert(item: $removal) { item in
            Alert(title: Text(String(localized: "Remove")), message: Text(verbatim: item.package.source + "\n" + item.profile.title),
                  primaryButton: .destructive(Text(String(localized: "Remove"))) { mutate(item.profile, action: "remove", source: item.package.source) },
                  secondaryButton: .cancel())
        }
    }

    private func packageRow(_ package: Package, in profile: Profile) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: package.pages.isEmpty ? "puzzlepiece.extension" : "rectangle.on.rectangle")
                .foregroundStyle(.secondary).frame(width: 24, height: 30)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                if package.pages.isEmpty {
                    Text(verbatim: package.source).font(.headline).textSelection(.enabled)
                    Text(verbatim: profile.title).font(.caption).foregroundStyle(.secondary)
                } else {
                    ForEach(package.pages) { page in
                        Button {
                            openPage(MacDesktopPluginPage(id: page.id, title: page.title, package: package.source))
                        } label: {
                            Text(verbatim: page.title).font(.headline).lineLimit(2)
                                .frame(maxWidth: .infinity, minHeight: 30, alignment: .leading)
                                .padding(.horizontal, 6).contentShape(Rectangle())
                        }
                        .buttonStyle(MacSidebarNavigationButtonStyle(isSelected: false))
                        .accessibilityIdentifier("workspace.plugins.open." + page.id)
                    }
                    Text(verbatim: package.source + " · " + profile.title)
                        .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                        .padding(.horizontal, 6)
                }
            }
            Spacer(minLength: 0)
            Menu {
                if let links = package.links, !links.isEmpty {
                    ForEach(Array(links.enumerated()), id: \.offset) { _, link in
                        Link(destination: link.url) {
                            Label(link.title + " · " + (link.url.host ?? ""), systemImage: "arrow.up.right.square")
                        }
                    }
                    Divider()
                }
                Button(String(localized: "Remove"), role: .destructive) { removal = Removal(profile: profile, package: package) }
                    .disabled(updating || !profile.available)
            } label: { Image(systemName: "ellipsis").frame(width: 28, height: 28) }
            .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
            .accessibilityLabel(String(localized: "More") + " · " + package.source)
        }
        .padding(.vertical, 12)
    }

    private func errorMessage(_ message: String) -> some View {
        Text(verbatim: message).font(.callout).foregroundStyle(MacDesignColor.danger).textSelection(.enabled)
    }

    private func refresh() {
        NotificationCenter.default.post(name: .macDesktopPluginsChanged, object: nil)
    }

    private func mutate(_ profile: Profile, action: String, source input: String) {
        guard !updating else { return }
        updating = true
        failure = nil
        Task { @MainActor in
            defer { updating = false; refresh() }
            do {
                _ = try await client.manage(runtime: profile.runtime, profile: profile.profile, action: action, source: input)
                if action == "install" { source = ""; adding = false }
            } catch { failure = error.localizedDescription }
        }
    }
}

struct MacAddPluginSheet: View {
    typealias Profile = MacDesktopPluginLibrary.Profile
    let profiles: [Profile]
    @Binding var source: String
    @Binding var destination: String?
    let updating: Bool
    let failure: String?
    let refresh: () -> Void
    let cancel: () -> Void
    let install: (Profile, String) -> Void
    private var selected: Profile? {
        profiles.first { $0.id == destination } ?? profiles.first { $0.available && $0.error == nil } ?? profiles.first
    }
    private var installableSource: String { source.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Add plugin").font(.title2.weight(.semibold))
            LabeledContent(String(localized: "This Mac")) {
                Picker(String(localized: "Runtime"), selection: Binding(get: { selected?.id ?? "" }, set: { destination = $0 })) {
                    ForEach(profiles) { Text(verbatim: $0.title).tag($0.id) }
                }.labelsHidden().fixedSize().accessibilityIdentifier("workspace.plugins.destination")
            }
            VStack(alignment: .leading, spacing: 6) {
                Text("Source").font(.headline)
                HStack(spacing: 8) {
                    TextField(String(localized: "Package name or folder"), text: $source)
                        .accessibilityIdentifier("workspace.plugins.source")
                    Button(String(localized: "Choose Folder")) {
                        let panel = NSOpenPanel()
                        panel.canChooseDirectories = true
                        panel.canChooseFiles = false
                        panel.allowsMultipleSelection = false
                        if panel.runModal() == .OK, let folder = panel.url { source = folder.path }
                    }
                }
            }
            if let selected, !selected.available {
                Text(String(format: String(localized: "Install %@ on this Mac to manage its packages."),
                            selected.runtime == "pi" ? "Pi" : "DeepSeek Harness"))
                    .font(.callout).foregroundStyle(.secondary)
                    .accessibilityIdentifier("workspace.plugins.missingRuntime")
                HStack(spacing: 14) {
                    if let guide = selected.installationGuide {
                        Link(String(localized: "Installation guide", table: "ModelCenter"), destination: guide)
                            .accessibilityIdentifier("workspace.plugins.runtimeGuide")
                    }
                    Button(String(localized: "Refresh"), action: refresh)
                        .accessibilityIdentifier("workspace.plugins.refreshRuntime")
                }
            } else if let error = selected?.error {
                Text(verbatim: error).font(.callout).foregroundStyle(MacDesignColor.danger).textSelection(.enabled)
            }
            if let failure {
                Text(verbatim: failure).font(.callout).foregroundStyle(MacDesignColor.danger).textSelection(.enabled)
            }
            Text("This package can run code on your Mac.").font(.callout).foregroundStyle(.secondary)
            HStack {
                if updating { ProgressView().controlSize(.small) }
                Spacer()
                Button(String(localized: "Cancel"), action: cancel).keyboardShortcut(.cancelAction)
                    .accessibilityIdentifier("workspace.plugins.cancelAdd")
                Button(String(localized: "Install plugin")) {
                    if let selected { install(selected, installableSource) }
                }
                .buttonStyle(.nolaProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(installableSource.isEmpty || selected?.available != true || selected?.error != nil)
                .accessibilityIdentifier("workspace.plugins.confirmInstall")
            }
        }
        .disabled(updating)
        .interactiveDismissDisabled(updating)
        .padding(24)
        .frame(width: 410)
        .background(MacDesignColor.canvas)
    }

}
