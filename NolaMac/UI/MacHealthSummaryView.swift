// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Displays aggregate iPhone Health data synced through iCloud KVS.

import AppKit
import Charts
import SwiftUI

private func macHealthText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    guard keyed == key else { return keyed }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

struct MacHealthSummaryView: View {
    @State private var store = MacHealthSyncStore.shared
    @State private var syncFeedback: MacHealthSyncFeedback?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let navigationAction: (() -> Void)?

    init(navigationAction: (() -> Void)? = nil) {
        self.navigationAction = navigationAction
    }

    var body: some View {
        VStack(spacing: 0) {
            MacPageHeader(
                LocalizedStringKey(macHealthText("mac.health.title", "iPhone Health")),
                symbol: "heart",
                navigationAction: navigationAction,
                navigationSymbol: "chevron.left",
                navigationLabel: LocalizedStringKey(macHealthText("mac.health.data_center", "Data Center"))
            ) {
                if let snapshot = store.snapshot {
                    Text(String(format: macHealthText("mac.health.synced_from", "Synced from %@ · %@"), snapshot.sourceDevice, snapshot.generatedAt.formatted(date: .abbreviated, time: .shortened)))
                } else if !store.cloudSyncAvailable {
                    Text(macHealthText("mac.health.sync_requires_signing", "iCloud Health sync requires a signed build with the KVS capability"))
                } else {
                    Text(macHealthText("mac.health.waiting", "Waiting for an aggregate Health snapshot from Nola on iPhone"))
                }
            } actions: {
                Button {
                    Task { await requestSync() }
                } label: {
                    if store.isSyncing { ProgressView().controlSize(.small) }
                    else { Label(macHealthText("mac.health.sync_now", "Sync Now"), systemImage: "arrow.triangle.2.circlepath") }
                }
                .disabled(store.isSyncing || !store.cloudSyncAvailable)
            }

            if let syncFeedback {
                HStack(spacing: 9) {
                    Image(systemName: syncFeedback.symbol)
                        .foregroundStyle(syncFeedback.tint)
                    Text(syncFeedback.message)
                        .font(.system(size: 11.5, weight: .medium))
                    Spacer(minLength: 8)
                    Button {
                        withAnimation(reduceMotion ? nil : .easeOut(duration: 0.16)) { self.syncFeedback = nil }
                    } label: {
                        Image(systemName: "xmark")
                            .font(.system(size: 9, weight: .semibold))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(L10n.close)
                }
                .padding(.horizontal, 14)
                .frame(minHeight: 38)
                .background(syncFeedback.tint.opacity(0.08))
                .overlay(alignment: .bottom) { Divider() }
                .transition(.move(edge: .top).combined(with: .opacity))
            }

            if let snapshot = store.snapshot {
                ScrollView {
                    VStack(alignment: .leading, spacing: 18) {
                        ViewThatFits(in: .horizontal) {
                            HStack {
                                activityDate(snapshot)
                                Spacer(minLength: 16)
                                freshness(snapshot)
                            }
                            VStack(alignment: .leading, spacing: 8) {
                                activityDate(snapshot)
                                freshness(snapshot)
                            }
                        }

                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 170), spacing: 10)], spacing: 10) {
                            metric(macHealthText("mac.health.steps", "Steps"), value: whole(snapshot.steps), unit: macHealthText("mac.health.unit.steps", "steps"), symbol: "figure.walk", tint: .blue)
                            metric(macHealthText("mac.health.distance", "Distance"), value: decimal(snapshot.distanceKilometers), unit: macHealthText("mac.health.unit.km", "km"), symbol: "location.fill", tint: .cyan)
                            metric(macHealthText("mac.health.active_energy", "Active Energy"), value: whole(snapshot.activeEnergyKilocalories), unit: macHealthText("mac.health.unit.kcal", "kcal"), symbol: "flame.fill", tint: .orange)
                            metric(macHealthText("mac.health.exercise", "Exercise"), value: whole(snapshot.exerciseMinutes), unit: macHealthText("mac.health.unit.min", "min"), symbol: "figure.run", tint: .green)
                            if let heart = snapshot.latestHeartRate {
                                metric(macHealthText("mac.health.heart_rate", "Heart Rate"), value: whole(heart), unit: macHealthText("mac.health.unit.bpm", "bpm"), symbol: "heart.fill", tint: .red)
                            }
                            if let resting = snapshot.restingHeartRate {
                                metric(macHealthText("mac.health.resting_heart_rate", "Resting Heart Rate"), value: whole(resting), unit: macHealthText("mac.health.unit.bpm", "bpm"), symbol: "heart.text.square", tint: .pink)
                            }
                            if let sleep = snapshot.sleepHours {
                                metric(macHealthText("mac.health.sleep", "Sleep"), value: decimal(sleep), unit: macHealthText("mac.health.unit.hr", "hr"), symbol: "bed.double.fill", tint: .indigo)
                            }
                        }

                        if !snapshot.dailySteps.isEmpty {
                            VStack(alignment: .leading, spacing: 10) {
                                Text(macHealthText("mac.health.seven_day_steps", "SEVEN-DAY STEPS"))
                                    .font(.system(size: 11, weight: .bold)).foregroundStyle(.secondary).tracking(0.8)
                                Chart(snapshot.dailySteps) { day in
                                    BarMark(
                                        x: .value(macHealthText("mac.health.chart.day", "Day"), day.date, unit: .day),
                                        y: .value(macHealthText("mac.health.steps", "Steps"), day.steps)
                                    )
                                    .foregroundStyle(Color.blue.gradient)
                                    .cornerRadius(3)
                                    .accessibilityLabel(day.date.formatted(date: .abbreviated, time: .omitted))
                                    .accessibilityValue("\(day.steps.formatted()) \(macHealthText("mac.health.steps", "Steps"))")
                                }
                                .chartXAxis { AxisMarks(values: .stride(by: .day)) { _ in AxisValueLabel(format: .dateTime.weekday(.narrow)) } }
                                .frame(height: 190)
                                .accessibilityLabel(macHealthText("mac.health.seven_day_steps", "Seven-day steps"))
                            }
                            .padding(15)
                            .macPanel()
                        }

                        if let workout = snapshot.latestWorkout {
                            ViewThatFits(in: .horizontal) {
                                HStack(spacing: 12) {
                                    workoutIdentity(workout)
                                    Spacer(minLength: 12)
                                    workoutEnergy(workout)
                                }
                                VStack(alignment: .leading, spacing: 10) {
                                    workoutIdentity(workout)
                                    workoutEnergy(workout)
                                }
                            }
                            .padding(14)
                            .macPanel()
                        }
                    }
                    .padding(18).frame(maxWidth: 900).frame(maxWidth: .infinity)
                }
            } else {
                ContentUnavailableView(
                    store.cloudSyncAvailable
                        ? macHealthText("mac.health.empty.title", "No Synced Health Summary")
                        : macHealthText("mac.health.unavailable.title", "iCloud Sync Unavailable"),
                    systemImage: store.cloudSyncAvailable ? "heart.text.square" : "icloud.slash",
                    description: Text(
                        store.cloudSyncAvailable
                            ? macHealthText("mac.health.empty.detail", "Open Nola’s Health dashboard on iPhone after granting Health access. The next successful refresh publishes an aggregate snapshot through iCloud.")
                            : macHealthText("mac.health.unavailable.detail", "Run a signed Nola build with the iCloud key-value store capability to receive Health summaries from iPhone.")
                    )
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(MacPageBackground())
    }

    private func metric(_ title: String, value: String, unit: String, symbol: String, tint: Color) -> some View {
        HStack(spacing: 11) {
            Image(systemName: symbol).foregroundStyle(tint).frame(width: 36, height: 36).background(tint.opacity(0.10), in: RoundedRectangle(cornerRadius: 10))
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.system(size: 10.5)).foregroundStyle(.secondary)
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text(value).font(.system(size: 17, weight: .semibold)).contentTransition(.numericText())
                    Text(unit).font(.system(size: 11)).foregroundStyle(.secondary)
                }
            }
            Spacer()
        }
        .padding(13)
        .macPanel()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityValue("\(value) \(unit)")
    }

    private func freshness(_ snapshot: HealthSummarySyncPayload) -> some View {
        let stale = !Calendar.current.isDateInToday(snapshot.activityDate)
            || Date().timeIntervalSince(snapshot.generatedAt) > 86_400
        let title = stale ? macHealthText("mac.health.stale", "May be outdated") : macHealthText("mac.health.fresh", "Up to date")
        return Label(title, systemImage: stale ? "clock.badge.exclamationmark" : "checkmark.circle.fill")
            .font(.system(size: 10.5, weight: .medium)).foregroundStyle(stale ? Color.orange : Color.green)
    }

    private func activityDate(_ snapshot: HealthSummarySyncPayload) -> some View {
        Label(activityDateTitle(snapshot.activityDate), systemImage: "calendar")
            .font(.system(size: 11.5, weight: .medium))
    }

    private func workoutIdentity(_ workout: HealthSummarySyncWorkout) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "figure.strengthtraining.traditional")
                .font(.system(size: 18, weight: .medium))
                .foregroundStyle(.green)
                .frame(width: 42, height: 42)
                .background(Color.green.opacity(0.10), in: RoundedRectangle(cornerRadius: 11))
            VStack(alignment: .leading, spacing: 3) {
                Text(workout.name)
                    .font(.system(size: 12.5, weight: .semibold))
                Text(String(
                    format: macHealthText("mac.health.workout_duration", "%@ min · %@"),
                    whole(workout.durationMinutes),
                    workout.startDate.formatted(date: .abbreviated, time: .shortened)
                ))
                .font(.system(size: 10.5))
                .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func workoutEnergy(_ workout: HealthSummarySyncWorkout) -> some View {
        if let energy = workout.energyKilocalories {
            Text(String(
                format: macHealthText("mac.health.workout_energy", "%@ kcal"),
                whole(energy)
            ))
            .fontWeight(.medium)
        }
    }

    private func activityDateTitle(_ date: Date) -> String {
        Calendar.current.isDateInToday(date) ? macHealthText("mac.health.today", "Today") : date.formatted(date: .abbreviated, time: .omitted)
    }

    private func whole(_ value: Double) -> String { value.formatted(.number.precision(.fractionLength(0))) }
    private func decimal(_ value: Double) -> String { value.formatted(.number.precision(.fractionLength(1))) }

    @MainActor
    private func requestSync() async {
        let previousDate = store.snapshot?.generatedAt
        await store.syncNow()
        let feedback: MacHealthSyncFeedback
        if let generatedAt = store.snapshot?.generatedAt,
           previousDate == nil || generatedAt > previousDate! {
            feedback = MacHealthSyncFeedback(
                message: macHealthText("mac.health.sync.updated", "Health summary updated"),
                symbol: "checkmark.circle.fill",
                tint: .green
            )
        } else if store.snapshot != nil {
            feedback = MacHealthSyncFeedback(
                message: macHealthText("mac.health.sync.up_to_date", "Health summary is already up to date"),
                symbol: "checkmark.circle",
                tint: .secondary
            )
        } else {
            feedback = MacHealthSyncFeedback(
                message: macHealthText("mac.health.sync.waiting_for_iphone", "Sync requested. Waiting for a Health summary from iPhone."),
                symbol: "icloud",
                tint: .blue
            )
        }
        withAnimation(reduceMotion ? nil : .easeOut(duration: 0.18)) { syncFeedback = feedback }
    }
}

private struct MacHealthSyncFeedback: Identifiable {
    let id = UUID()
    let message: String
    let symbol: String
    let tint: Color
}
