import type { ContextPack, DataSourceKind, DataSourceSnapshot, UnifiedDataSource } from './models';

export interface UploadedContextItem {
  name?: string;
  content?: string;
  kind?: string;
  enabled?: boolean;
}

export interface McpContextItem {
  name?: string;
  url?: string;
  endpoint?: string;
  enabled?: boolean;
}

export interface BuildContextDataSourcesInput {
  uploadedDocuments?: UploadedContextItem[];
  mcpServers?: McpContextItem[];
  includeHealthSummary?: boolean;
}

export interface BuildContextPackInput extends BuildContextDataSourcesInput {
  id: string;
  dataSources?: UnifiedDataSource[];
  createdAt?: number;
}

export function buildContextDataSources(input: BuildContextDataSourcesInput): UnifiedDataSource[] {
  const documents = input.uploadedDocuments ?? [];
  const mcpServers = input.mcpServers ?? [];
  const sources: UnifiedDataSource[] = [
    ...documents.map((item, index) => uploadedDocumentSource(item, index)),
    ...mcpServers.map((item, index) => mcpSource(item, index)),
  ];
  if (input.includeHealthSummary) {
    sources.push({
      id: 'health:local',
      name: 'Health',
      kind: 'health',
      capabilities: ['readable', 'contextual'],
      privacy: {
        scopes: ['local_storage', 'summary_only'],
        rawSamplesLeaveDevice: false,
      },
      connectionState: 'needs_permission',
    });
  }
  return sources;
}

export function buildContextPack(input: BuildContextPackInput): ContextPack {
  const dataSources = input.dataSources ?? buildContextDataSources(input);
  const uploadedDocuments = input.uploadedDocuments ?? [];
  const mcpServers = input.mcpServers ?? [];
  const snapshots: DataSourceSnapshot[] = dataSources
    .filter(source => (source.connectionState ?? 'ready') === 'ready')
    .map(source => snapshotForSource(source, uploadedDocuments, mcpServers))
    .filter((snapshot): snapshot is DataSourceSnapshot => snapshot !== undefined);
  return {
    id: input.id,
    dataSourceIds: snapshots.map(item => item.dataSourceId),
    snapshots,
    tokenEstimate: snapshots.reduce((sum, item) => sum + (item.tokenEstimate ?? 0), 0),
    createdAt: input.createdAt ?? Date.now(),
  };
}

function uploadedDocumentSource(item: UploadedContextItem, index: number): UnifiedDataSource {
  const name = nonBlank(item.name) ?? `Document ${index + 1}`;
  const content = item.content ?? '';
  const kind = uploadedKind(item.kind);
  return {
    id: `document:web:${index}:${stableIdPart(name)}`,
    name,
    kind,
    location: 'web',
    capabilities: ['readable', 'searchable', 'contextual'],
    privacy: {
      scopes: ['local_storage', 'allow_cloud_context'],
      rawSamplesLeaveDevice: false,
    },
    connectionState: item.enabled === false ? 'disabled' : 'ready',
    metadata: {
      characters: String(content.length),
      origin: 'web',
      originIndex: String(index),
    },
  };
}

function mcpSource(item: McpContextItem, index: number): UnifiedDataSource {
  const name = nonBlank(item.name) ?? `MCP ${index + 1}`;
  const location = nonBlank(item.url) ?? nonBlank(item.endpoint) ?? 'web';
  return {
    id: `mcp:web:${index}:${stableIdPart(name)}`,
    name,
    kind: 'mcp',
    location,
    capabilities: ['executable', 'contextual'],
    privacy: {
      scopes: ['allow_agent_tools', 'requires_confirmation'],
      rawSamplesLeaveDevice: false,
    },
    connectionState: item.enabled === false ? 'disabled' : 'ready',
    metadata: {
      origin: 'web',
      originIndex: String(index),
    },
  };
}

function snapshotForSource(
  source: UnifiedDataSource,
  uploadedDocuments: UploadedContextItem[],
  mcpServers: McpContextItem[],
): DataSourceSnapshot | undefined {
  const index = Number.parseInt(source.metadata?.originIndex ?? '-1', 10);
  if (!Number.isFinite(index) || index < 0) return undefined;
  let tokenEstimate = 0;
  if (source.kind === 'file' || source.kind === 'folder' || source.kind === 'uploaded_document') {
    tokenEstimate = estimateTokens((uploadedDocuments[index]?.content ?? '').slice(0, 4000));
  } else if (source.kind === 'mcp') {
    tokenEstimate = estimateTokens(JSON.stringify(mcpServers[index] ?? {}));
  } else {
    return undefined;
  }
  return {
    dataSourceId: source.id,
    itemCount: 1,
    tokenEstimate,
    capturedAt: Date.now(),
    summary: source.name,
  };
}

function uploadedKind(value?: string): DataSourceKind {
  const normalized = (value ?? '').trim().toLowerCase();
  if (normalized === 'folder') return 'folder';
  if (normalized === 'file') return 'file';
  return 'uploaded_document';
}

function nonBlank(value?: string): string | undefined {
  const trimmed = (value ?? '').trim();
  return trimmed ? trimmed : undefined;
}

function stableIdPart(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (!normalized) return 'untitled';
  return normalized
    .replace(/[^a-z0-9\u4e00-\u9fff]+/gi, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '')
    .slice(0, 48) || 'untitled';
}

function estimateTokens(value: string): number {
  let ascii = 0;
  let nonAscii = 0;
  for (const ch of value) {
    if (ch.charCodeAt(0) <= 127) ascii += 1;
    else nonAscii += 1;
  }
  return Math.floor(ascii / 4.7) + nonAscii;
}
