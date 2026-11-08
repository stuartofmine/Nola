// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DataSourcePermissionManager logic in the iOS service layer.
// - Interfaces:
//   - DataSourcePermissionManager
//   - AuthorizationStatus
//   - PermissionRequestResult
//   - PermissionError
// - APIs:
//   - none
//
//
//  DataSourcePermissionManager.swift
//  Nola
//
//

import Foundation
import Contacts
import EventKit
import HealthKit

public final class DataSourcePermissionManager {
    public static let shared = DataSourcePermissionManager()
    
    private let logger = Logger()
    
    private init() {}
    
    
    public func checkContactsPermission() -> AuthorizationStatus {
        let status = CNContactStore.authorizationStatus(for: .contacts)
        return AuthorizationStatus(from: status)
    }
    
    public func checkCalendarPermission() -> AuthorizationStatus {
        let status = EKEventStore.authorizationStatus(for: .event)
        return AuthorizationStatus(from: status)
    }
    
    public func checkRemindersPermission() -> AuthorizationStatus {
        let status = EKEventStore.authorizationStatus(for: .reminder)
        return AuthorizationStatus(from: status)
    }
    
    public func checkHealthPermission() -> AuthorizationStatus {
        guard HKHealthStore.isHealthDataAvailable() else {
            return .unavailable
        }

        // `authorizationStatus(for:)` reports write permission and is invalid
        // for this read-only data source. Track completion of the HealthKit
        // authorization flow; individual metric reads remain authoritative.
        return HealthDataSource.hasCompletedAuthorizationRequest ? .authorized : .notDetermined
    }
    
    public func checkAllPermissions() -> [DataSourceType: AuthorizationStatus] {
        return [
            .contacts: checkContactsPermission(),
            .calendars: checkCalendarPermission(),
            .reminders: checkRemindersPermission(),
            .health: checkHealthPermission(),
            .url: .authorized
        ]
    }
    
    
    public func requestContactsPermission() async throws -> Bool {
        let store = CNContactStore()
        
        logger.log(.info, "PermissionManager", "Requesting contacts permission")
        
        do {
            let granted = try await store.requestAccess(for: .contacts)
            logger.log(.info, "PermissionManager", "Contacts permission result", metadata: ["granted": granted ? "true" : "false"])
            return granted
        } catch {
            logger.log(.error, "PermissionManager", "Contacts permission error", metadata: ["error": error.localizedDescription])
            throw error
        }
    }
    
    public func requestCalendarPermission() async throws -> Bool {
        let store = EKEventStore()
        
        logger.log(.info, "PermissionManager", "Requesting calendar permission")
        
        do {
            let granted: Bool
            if #available(iOS 17.0, *) {
                granted = try await store.requestFullAccessToEvents()
            } else {
                granted = try await store.requestAccess(to: .event)
            }
            
            logger.log(.info, "PermissionManager", "Calendar permission result", metadata: ["granted": granted ? "true" : "false"])
            return granted
        } catch {
            logger.log(.error, "PermissionManager", "Calendar permission error", metadata: ["error": error.localizedDescription])
            throw error
        }
    }
    
    public func requestRemindersPermission() async throws -> Bool {
        let store = EKEventStore()
        
        logger.log(.info, "PermissionManager", "Requesting reminders permission")
        
        do {
            let granted: Bool
            if #available(iOS 17.0, *) {
                granted = try await store.requestFullAccessToReminders()
            } else {
                granted = try await store.requestAccess(to: .reminder)
            }
            
            logger.log(.info, "PermissionManager", "Reminders permission result", metadata: ["granted": granted ? "true" : "false"])
            return granted
        } catch {
            logger.log(.error, "PermissionManager", "Reminders permission error", metadata: ["error": error.localizedDescription])
            throw error
        }
    }
    
    public func requestHealthPermission(types: Set<HKObjectType>? = nil) async throws -> Bool {
        guard HKHealthStore.isHealthDataAvailable() else {
            logger.log(.warn, "PermissionManager", "HealthKit not available")
            throw PermissionError.healthKitUnavailable
        }
        let readTypes = types ?? makeDefaultHealthTypes()
        let healthSource = HealthDataSource()
        
        logger.log(.info, "PermissionManager", "Requesting health permission", metadata: ["typeCount": String(readTypes.count)])

        do {
            let granted = try await healthSource.requestAuthorization(read: readTypes)
            logger.log(.info, "PermissionManager", "Health permission result", metadata: [
                "authorized": granted ? "true" : "false"
            ])
            return granted
        } catch {
            logger.log(.error, "PermissionManager", "Health permission error", metadata: ["error": error.localizedDescription])
            throw error
        }
    }
    
    public func requestAllPermissions() async -> PermissionRequestResult {
        var results: [DataSourceType: Bool] = [:]
        var errors: [DataSourceType: Error] = [:]
        
        do {
            results[.contacts] = try await requestContactsPermission()
        } catch {
            results[.contacts] = false
            errors[.contacts] = error
        }
        
        do {
            results[.calendars] = try await requestCalendarPermission()
        } catch {
            results[.calendars] = false
            errors[.calendars] = error
        }
        
        do {
            results[.reminders] = try await requestRemindersPermission()
        } catch {
            results[.reminders] = false
            errors[.reminders] = error
        }
        
        do {
            results[.health] = try await requestHealthPermission()
        } catch {
            results[.health] = false
            errors[.health] = error
        }
        
        return PermissionRequestResult(results: results, errors: errors)
    }
    
    
    private func makeDefaultHealthTypes() -> Set<HKObjectType> {
        var types = Set<HKObjectType>()
        
        if let stepType = HKObjectType.quantityType(forIdentifier: .stepCount) {
            types.insert(stepType)
        }
        if let distanceType = HKObjectType.quantityType(forIdentifier: .distanceWalkingRunning) {
            types.insert(distanceType)
        }
        if let activeEnergyType = HKObjectType.quantityType(forIdentifier: .activeEnergyBurned) {
            types.insert(activeEnergyType)
        }
        if let exerciseType = HKObjectType.quantityType(forIdentifier: .appleExerciseTime) {
            types.insert(exerciseType)
        }
        if let sleepType = HKObjectType.categoryType(forIdentifier: .sleepAnalysis) {
            types.insert(sleepType)
        }
        if let heartRateType = HKObjectType.quantityType(forIdentifier: .heartRate) {
            types.insert(heartRateType)
        }
        if let restingHeartRateType = HKObjectType.quantityType(forIdentifier: .restingHeartRate) {
            types.insert(restingHeartRateType)
        }
        
        types.insert(HKObjectType.workoutType())
        
        return types
    }
    
    public func permissionStatusDescription(for type: DataSourceType) -> String {
        let status = getPermissionStatus(for: type)
        return "\(type.displayName): \(status.displayText)"
    }
    
    public func getPermissionStatus(for type: DataSourceType) -> AuthorizationStatus {
        switch type {
        case .contacts: return checkContactsPermission()
        case .calendars: return checkCalendarPermission()
        case .reminders: return checkRemindersPermission()
        case .health: return checkHealthPermission()
        case .url: return .authorized
        }
    }
    
    public func areAllRequiredPermissionsGranted() -> Bool {
        let statuses = checkAllPermissions()
        
        let required: [DataSourceType] = [.contacts, .calendars, .reminders]
        
        return required.allSatisfy { type in
            if let status = statuses[type] {
                return status == .authorized
            }
            return false
        }
    }
    
    public func getUnauthorizedDataSources() -> [DataSourceType] {
        let statuses = checkAllPermissions()
        
        return statuses.filter { _, status in
            status == .notDetermined || status == .denied || status == .insufficient
        }.map { $0.key }
    }
}

// MARK: - Supporting Types

public enum AuthorizationStatus {
    case notDetermined
    case authorized
    case insufficient
    case denied
    case restricted
    case unavailable
    
    init(from status: CNAuthorizationStatus) {
        switch status {
        case .notDetermined: self = .notDetermined
        case .authorized: self = .authorized
        case .limited: self = .insufficient
        case .denied: self = .denied
        case .restricted: self = .restricted
        @unknown default: self = .notDetermined
        }
    }
    
    init(from status: EKAuthorizationStatus) {
        switch status {
        case .notDetermined: self = .notDetermined
        case .fullAccess: self = .authorized
        case .denied: self = .denied
        case .restricted: self = .restricted
        case .writeOnly: self = .insufficient
        @unknown default: self = .notDetermined
        }
    }
    
    init(from status: HKAuthorizationStatus) {
        switch status {
        case .notDetermined: self = .notDetermined
        case .sharingAuthorized: self = .authorized
        case .sharingDenied: self = .denied
        @unknown default: self = .notDetermined
        }
    }
    
    public var displayText: String {
        switch self {
        case .notDetermined: return NSLocalizedString("permission.status.not_requested", value: "Not requested", comment: "Permission status")
        case .authorized: return NSLocalizedString("autogen.DataSourcePermissionManager.284cc0ba", comment: "AUTOGEN")
        case .insufficient: return NSLocalizedString("autogen.DataSourcePermissionManager.28183c49", comment: "AUTOGEN")
        case .denied: return NSLocalizedString("autogen.DataSourcePermissionManager.4c7c52c7", comment: "AUTOGEN")
        case .restricted: return NSLocalizedString("autogen.DataSourcePermissionManager.93633305", comment: "AUTOGEN")
        case .unavailable: return NSLocalizedString("autogen.DataSourcePermissionManager.beff4a1c", comment: "AUTOGEN")
        }
    }
    
    public var color: String {
        switch self {
        case .notDetermined: return "gray"
        case .authorized: return "green"
        case .insufficient: return "orange"
        case .denied: return "red"
        case .restricted: return "orange"
        case .unavailable: return "gray"
        }
    }
    
    public var isAuthorized: Bool {
        return self == .authorized
    }
}

public struct PermissionRequestResult {
    public let results: [DataSourceType: Bool]
    public let errors: [DataSourceType: Error]
    
    public var allGranted: Bool {
        return results.values.allSatisfy { $0 }
    }
    
    public var grantedCount: Int {
        return results.values.filter { $0 }.count
    }
    
    public var deniedCount: Int {
        return results.values.filter { !$0 }.count
    }
    
    public var hasErrors: Bool {
        return !errors.isEmpty
    }
}

public enum PermissionError: Error, LocalizedError {
    case healthKitUnavailable
    case permissionDenied(source: String)
    case requestFailed(source: String, reason: String)
    
    public var errorDescription: String? {
        switch self {
        case .healthKitUnavailable:
            return NSLocalizedString("autogen.DataSourcePermissionManager.30b06e2f", comment: "AUTOGEN")
        case .permissionDenied(let source):
            return "\(source) 权限被拒绝"
        case .requestFailed(let source, let reason):
            return "\(source) 权限请求失败: \(reason)"
        }
    }
}
