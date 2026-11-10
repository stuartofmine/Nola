// SPDX-License-Identifier: Apache-2.0
// Typed Health dashboard UI.

import Charts
import Combine
import HealthKit
import SwiftUI

@MainActor
final class HealthDashboardStore: ObservableObject {
    typealias SnapshotLoader = @MainActor () async throws -> HealthDashboardSnapshot

    static let shared = HealthDashboardStore()

    @Published private(set) var snapshot: HealthDashboardSnapshot?
    @Published private(set) var isLoading = false
    @Published private(set) var errorMessage: String?

    private let loadSnapshot: SnapshotLoader
    private var activeLoadTask: Task<Void, Never>?

    init(loadSnapshot: @escaping SnapshotLoader = {
        try await HealthDashboardRepository().fetchSnapshot()
    }) {
        self.loadSnapshot = loadSnapshot
    }

    func load() async {
        if let activeLoadTask {
            await activeLoadTask.value
            return
        }

        isLoading = true
        errorMessage = nil
        let task = Task { @MainActor [weak self] in
            guard let self else { return }
            await self.performLoad()
        }
        activeLoadTask = task
        await task.value
        activeLoadTask = nil
        isLoading = false
    }

    private func performLoad() async {
        do {
            let fetched = try await loadSnapshot()
            snapshot = fetched
            HealthSummarySync.publish(HealthSummarySyncPayload(
                schemaVersion: 1,
                sourceDevice: "iPhone",
                generatedAt: fetched.generatedAt,
                activityDate: fetched.activityDate,
                steps: fetched.todaySteps,
                distanceKilometers: fetched.todayDistanceKilometers,
                activeEnergyKilocalories: fetched.todayActiveEnergyKilocalories,
                exerciseMinutes: fetched.todayExerciseMinutes,
                latestHeartRate: fetched.latestHeartRate,
                latestHeartRateDate: fetched.latestHeartRateDate,
                restingHeartRate: fetched.restingHeartRate,
                sleepHours: fetched.sleepHours,
                sleepEndDate: fetched.sleepEndDate,
                latestWorkout: fetched.latestWorkout.map {
                    HealthSummarySyncWorkout(
                        name: $0.name,
                        startDate: $0.startDate,
                        durationMinutes: $0.durationMinutes,
                        energyKilocalories: $0.energyKilocalories
                    )
                },
                dailySteps: fetched.dailySteps.map {
                    HealthSummarySyncDay(date: $0.date, steps: $0.steps)
                }
            ))
        } catch is CancellationError {
            return
        } catch let error as HKError where error.code == .errorAuthorizationNotDetermined {
            // Initial permission is a waiting state, not a failed refresh.
            // The surrounding source card provides the explicit authorization action.
            snapshot = nil
            errorMessage = nil
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}

struct HealthDashboardView: View {
    @ObservedObject var store: HealthDashboardStore
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    private var activityColumns: [GridItem] {
        Array(repeating: GridItem(.flexible()), count: dynamicTypeSize.isAccessibilitySize ? 1 : 2)
    }

    var body: some View {
        Group {
            if let snapshot = store.snapshot {
                dashboard(snapshot)
            } else if store.isLoading {
                loadingView
            } else if let error = store.errorMessage {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Text(tr("health.dashboard.no_readable_data", "No readable Health data is available yet."))
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
        .animation(reduceMotion ? nil : DesignTokens.revealAnimation, value: store.isLoading)
    }

    private func dashboard(_ snapshot: HealthDashboardSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 18) {
            if let error = store.errorMessage {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text(activityTitle(snapshot.activityDate))
                        .font(.title3.weight(.semibold))
                    Text(snapshot.generatedAt, style: .time)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if store.isLoading { ProgressView().controlSize(.small) }
            }

            LazyVGrid(columns: activityColumns, spacing: 10) {
                metricCard(tr("health.dashboard.steps", "Steps"), value: integer(snapshot.todaySteps), unit: tr("health.dashboard.unit.steps", "steps"), icon: "figure.walk", color: .blue)
                metricCard(tr("health.dashboard.distance", "Distance"), value: decimal(snapshot.todayDistanceKilometers), unit: tr("health.dashboard.unit.km", "km"), icon: "location.fill", color: .cyan)
                metricCard(tr("health.dashboard.active_energy", "Active energy"), value: integer(snapshot.todayActiveEnergyKilocalories), unit: tr("health.dashboard.unit.kcal", "kcal"), icon: "flame.fill", color: .orange)
                metricCard(tr("health.dashboard.exercise", "Exercise"), value: integer(snapshot.todayExerciseMinutes), unit: tr("health.dashboard.unit.min", "min"), icon: "figure.run", color: .green)
            }

            if !snapshot.dailySteps.isEmpty {
                VStack(alignment: .leading, spacing: 10) {
                    Text(tr("health.dashboard.last_7_days", "Last 7 days"))
                        .font(.headline)
                    Chart(snapshot.dailySteps) { item in
                        BarMark(
                            x: .value(tr("health.dashboard.day", "Day"), item.date, unit: .day),
                            y: .value(tr("health.dashboard.steps", "Steps"), item.steps)
                        )
                        .foregroundStyle(
                            LinearGradient(colors: [.blue.opacity(0.55), .blue], startPoint: .bottom, endPoint: .top)
                        )
                        .cornerRadius(4)
                    }
                    .chartXAxis {
                        AxisMarks(values: .stride(by: .day)) { value in
                            AxisValueLabel(format: .dateTime.weekday(.narrow))
                        }
                    }
                    .chartYAxis {
                        AxisMarks(position: .leading) { value in
                            AxisGridLine().foregroundStyle(.quaternary)
                            AxisValueLabel {
                                if let steps = value.as(Double.self) {
                                    Text(steps >= 1_000 ? "\(Int(steps / 1_000))k" : "\(Int(steps))")
                                }
                            }
                        }
                    }
                    .frame(height: 150)
                }
                .padding(14)
                .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            }

            VStack(spacing: 0) {
                detailRow(
                    title: tr("health.dashboard.latest_heart_rate", "Latest heart rate"),
                    value: snapshot.latestHeartRate.map { String(format: tr("health.dashboard.value_bpm_format", "%@ bpm"), integer($0)) } ?? tr("health.dashboard.no_recent_data", "No recent data"),
                    subtitle: snapshot.latestHeartRateDate.map(relativeDate),
                    icon: "heart.fill",
                    color: .red
                )
                Divider().padding(.leading, 42)
                detailRow(
                    title: tr("health.dashboard.resting_heart_rate", "Resting heart rate"),
                    value: snapshot.restingHeartRate.map { String(format: tr("health.dashboard.value_bpm_format", "%@ bpm"), integer($0)) } ?? tr("health.dashboard.no_recent_data", "No recent data"),
                    subtitle: nil,
                    icon: "waveform.path.ecg",
                    color: .pink
                )
                Divider().padding(.leading, 42)
                detailRow(
                    title: tr("health.dashboard.recent_sleep", "Recent sleep"),
                    value: snapshot.sleepHours.map { String(format: tr("health.dashboard.value_hours_format", "%@ hr"), decimal($0)) } ?? tr("health.dashboard.no_recent_data", "No recent data"),
                    subtitle: snapshot.sleepEndDate.map(relativeDate),
                    icon: "bed.double.fill",
                    color: .teal
                )
                Divider().padding(.leading, 42)
                if let workout = snapshot.latestWorkout {
                    detailRow(
                        title: workout.name,
                        value: String(format: tr("health.dashboard.value_minutes_format", "%@ min"), integer(workout.durationMinutes)),
                        subtitle: workout.energyKilocalories.map { String(format: tr("health.dashboard.value_energy_date_format", "%1$@ kcal · %2$@"), integer($0), relativeDate(workout.startDate)) }
                            ?? relativeDate(workout.startDate),
                        icon: "figure.run.circle.fill",
                        color: .green
                    )
                } else {
                    detailRow(title: tr("health.dashboard.recent_workout", "Recent workout"), value: tr("health.dashboard.no_workout_30_days", "No workout in 30 days"), subtitle: nil, icon: "figure.run.circle", color: .secondary)
                }
            }
            .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        }
    }

    private func metricCard(_ title: String, value: String, unit: String, icon: String, color: Color) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Image(systemName: icon)
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(color)
                .frame(width: 32, height: 32)
                .background(color.opacity(0.12), in: Circle())
            VStack(alignment: .leading, spacing: 2) {
                Text(value)
                    .font(.system(.title2, design: .rounded, weight: .semibold))
                    .contentTransition(.numericText())
                Text("\(title) · \(unit)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
    }

    private func detailRow(title: String, value: String, subtitle: String?, icon: String, color: Color) -> some View {
        HStack(spacing: 12) {
            Image(systemName: icon)
                .foregroundStyle(color)
                .frame(width: 30)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.subheadline.weight(.medium))
                if let subtitle { Text(subtitle).font(.caption).foregroundStyle(.secondary) }
            }
            Spacer()
            Text(value)
                .font(.subheadline.monospacedDigit())
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.trailing)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }

    private var loadingView: some View {
        VStack(spacing: 12) {
            ProgressView()
            Text(tr("health.dashboard.reading", "Reading the latest Health data…"))
                .font(.callout)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 30)
    }

    private func integer(_ value: Double) -> String { value.formatted(.number.precision(.fractionLength(0))) }
    private func decimal(_ value: Double) -> String { value.formatted(.number.precision(.fractionLength(1))) }
    private func relativeDate(_ date: Date) -> String { date.formatted(.relative(presentation: .named)) }
    private func activityTitle(_ date: Date) -> String {
        if Calendar.autoupdatingCurrent.isDateInToday(date) {
            return tr("health.dashboard.today", "Today")
        }
        return date.formatted(date: .abbreviated, time: .omitted)
    }
    private func tr(_ key: String, _ fallback: String) -> String {
        NSLocalizedString(key, value: fallback, comment: "")
    }
}
