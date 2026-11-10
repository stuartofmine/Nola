import XCTest
@testable import Nola

@MainActor
final class DataSourceStatusStoreTests: XCTestCase {
    func testSuccessfulHealthReadOverridesOpaqueNotDeterminedStatus() {
        let generatedAt = Date()
        let snapshot = HealthDashboardSnapshot(
            generatedAt: generatedAt,
            activityDate: generatedAt,
            todaySteps: 8_432,
            todayDistanceKilometers: 6.2,
            todayActiveEnergyKilocalories: 418,
            todayExerciseMinutes: 36,
            latestHeartRate: 72,
            latestHeartRateDate: generatedAt,
            restingHeartRate: 58,
            sleepHours: 7.4,
            sleepEndDate: generatedAt,
            latestWorkout: nil,
            dailySteps: [HealthDayValue(date: generatedAt, steps: 8_432)]
        )

        let state = DataSourceStatusStore.resolveHealthState(
            snapshot: snapshot,
            errorMessage: nil,
            reportedPermission: .notDetermined,
            cached: nil
        )

        XCTAssertEqual(state.phase, .ready)
        XCTAssertEqual(state.permission, .authorized)
        XCTAssertEqual(state.itemCount, snapshot.availableMetricCount)
        XCTAssertEqual(state.lastReadAt, generatedAt)
    }

    func testFailedHealthReadStillRequestsUndeterminedPermission() {
        let state = DataSourceStatusStore.resolveHealthState(
            snapshot: nil,
            errorMessage: "Health data is not readable",
            reportedPermission: .notDetermined,
            cached: nil
        )

        XCTAssertEqual(state.phase, .needsPermission)
        XCTAssertEqual(state.permission, .notDetermined)
        XCTAssertNil(state.lastReadAt)
    }

    func testEmptyHealthSnapshotDoesNotMasqueradeAsAuthorizedData() {
        let generatedAt = Date()
        let snapshot = HealthDashboardSnapshot(
            generatedAt: generatedAt,
            activityDate: generatedAt,
            todaySteps: 0,
            todayDistanceKilometers: 0,
            todayActiveEnergyKilocalories: 0,
            todayExerciseMinutes: 0,
            latestHeartRate: nil,
            latestHeartRateDate: nil,
            restingHeartRate: nil,
            sleepHours: nil,
            sleepEndDate: nil,
            latestWorkout: nil,
            dailySteps: [HealthDayValue(date: generatedAt, steps: 0)]
        )

        let state = DataSourceStatusStore.resolveHealthState(
            snapshot: snapshot,
            errorMessage: nil,
            reportedPermission: .notDetermined,
            cached: nil
        )

        XCTAssertFalse(snapshot.hasReadableData)
        XCTAssertEqual(snapshot.availableMetricCount, 0)
        XCTAssertEqual(state.phase, .needsPermission)
        XCTAssertEqual(state.permission, .notDetermined)
        XCTAssertNil(state.itemCount)
        XCTAssertNil(state.lastReadAt)
    }
}
