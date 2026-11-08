// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements EventDataContext logic in the project module.
// - Interfaces:
//   - EventDataContext
// - APIs:
//   - none
//
export interface EventDataContext {
  fetchCalendars(): Promise<any[]>;
  fetchReminders(): Promise<any[]>;
  createCalendarEvent(title: string, start: number, end: number): Promise<string>;
  createReminder(title: string, dueDate: number): Promise<string>;
}

export default EventDataContext;
