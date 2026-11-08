// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements models logic in the project module.
// - Interfaces:
//   - URLDataSource
//   - HistoryEntry
//   - SummaryItem
// - APIs:
//   - none
//
export interface URLDataSource {
  id: string;
  name?: string;
  url: string;
  history?: HistoryEntry[];
}

export type DataSourceKind =
  | 'file'
  | 'folder'
  | 'database'
  | 'url'
  | 'web_search'
  | 'api'
  | 'health'
  | 'calendar'
  | 'reminder'
  | 'mcp'
  | 'uploaded_document'
  | 'web_host'
  | 'custom';

export type DataSourceCapability =
  | 'readable'
  | 'searchable'
  | 'watchable'
  | 'schedulable'
  | 'writable'
  | 'executable'
  | 'contextual';

export type DataSourcePrivacyScope =
  | 'local_storage'
  | 'summary_only'
  | 'allow_cloud_context'
  | 'allow_agent_tools'
  | 'allow_write_back'
  | 'requires_confirmation';

export type DataSourceConnectionState =
  | 'ready'
  | 'disabled'
  | 'needs_permission'
  | 'unavailable'
  | 'error';

export interface DataSourcePrivacyPolicy {
  scopes: DataSourcePrivacyScope[];
  rawSamplesLeaveDevice?: boolean;
}

export interface UnifiedDataSource {
  id: string;
  name: string;
  kind: DataSourceKind;
  location?: string;
  capabilities: DataSourceCapability[];
  privacy: DataSourcePrivacyPolicy;
  connectionState?: DataSourceConnectionState;
  lastSyncedAt?: number;
  metadata?: Record<string, string>;
}

export interface DataSourceSnapshot {
  dataSourceId: string;
  itemCount?: number;
  tokenEstimate?: number;
  capturedAt: number;
  cursor?: string;
  summary?: string;
}

export interface ContextPack {
  id: string;
  dataSourceIds: string[];
  snapshots?: DataSourceSnapshot[];
  tokenEstimate?: number;
  createdAt: number;
}

export interface HistoryEntry {
  id: string;
  title?: string;
  body?: string;
  createdAt: number; // epoch ms
  eventId?: string;
}

export interface SummaryItem {
  dedupeKey?: string;
  title: string;
  body?: string;
  priority?: 'low' | 'medium' | 'high';
}
