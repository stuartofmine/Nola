// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements SystemPermissionsContext logic in the project module.
// - Interfaces:
//   - SystemPermissionsContext
// - APIs:
//   - none
//
/**
 * Valdi @Context interface stub for system permissions.
 * Native implementations should be generated via Valdi annotations.
 */
export interface SystemPermissionsContext {
  requestContactsAccess(): Promise<boolean>;
  requestCalendarAccess(): Promise<boolean>;
  requestReminderAccess(): Promise<boolean>;
  requestHealthAccess(): Promise<boolean>;
}

export default SystemPermissionsContext;
