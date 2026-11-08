// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines entry-point wiring and request flow for the Cloudflare Worker backend.
// - Interfaces:
//   - JsonObject
//   - Env
//   - Config
//   - AuthContext
//   - TEXT_HEADERS
//   - JSON_HEADERS
//   - ALLOWED_HTTP_METHODS
// - APIs:
//   - /healthz
//   - /admin/outbox/dlq/count
//   - /admin/outbox/dlq/replay
//   - /search/serper
//
import { readReplicaBody, replicaPageEnd, ReplicaTransportError } from "./replica_budget";

type JsonObject = Record<string, unknown>;

interface Env {
  DB: D1Database;
  nola_backend_env?: string | JsonObject;
  NOLA_BACKEND_ENV?: string | JsonObject;
  [key: string]: unknown;
}

interface Config {
  raw: JsonObject;
  get(name: string, fallback?: string): string;
  bool(name: string, fallback?: boolean): boolean;
  int(name: string, fallback: number): number;
}

interface AuthContext {
  clientUuid: string;
  subject: string;
}

const TEXT_HEADERS = { "content-type": "text/plain; charset=utf-8" };
const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };
const EMAIL_RE = /^[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+$/;
const PASSWORD_MIN_LEN = 8;
const PASSWORD_HASH_ITERATIONS = 100_000;
const ALLOWED_HTTP_METHODS = new Set(["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD"]);
const SCAN_ALLOWED_PREFIXES = ["/", "/healthz", "/search", "/oauth", "/v1", "/api", "/admin"];
const SCAN_SUSPICIOUS_PATH_MARKERS = [
  "/.env",
  "/.git",
  "wp-admin",
  "wp-login",
  "phpmyadmin",
  "cgi-bin",
  ".php",
  ".aspx",
  "/actuator",
  "/jenkins",
  "/vendor/",
];
const scanEventsByIp = new Map<string, number[]>();
const scanBanUntilByIp = new Map<string, number>();

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    try {
      return await handleRequest(request, env, ctx);
    } catch (error) {
      const detail = error instanceof HttpError ? error.detail : error instanceof Error ? error.message : String(error);
      const status = error instanceof HttpError ? error.status : 500;
      return json({ detail }, status);
    }
  },
  async scheduled(event: ScheduledEvent, env: Env, ctx: ExecutionContext): Promise<void> {
    const config = loadConfig(env);
    if (!config.bool("PUSH_FEATURE_ENABLED", false) || !config.bool("PUSH_SCHEDULER_ENABLED", false)) return;
    ctx.waitUntil(runDueScheduledSources(env, config, new Date(event.scheduledTime)));
  },
};

interface MessagingChannelConnectionRow extends JsonObject {
  connection_id: string;
  client_uuid: string;
  kind: string;
  display_name: string;
  account_id: string;
  secret_ref: string;
  credential_ref: string;
  webhook_secret_ref: string;
  verify_token_ref: string;
  provider_config_json: string;
  enabled: number;
  state: string;
  last_received_at?: string;
  last_error?: string;
}

class HttpError extends Error {
  constructor(public status: number, public detail: string) {
    super(detail);
  }
}

async function listMessagingChannels(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  const rows = await env.DB.prepare("SELECT connection_id, kind, display_name, account_id, credential_ref, webhook_secret_ref, verify_token_ref, provider_config_json, enabled, state, last_received_at, last_error, created_at, updated_at FROM messagingchannelconnectionrecord WHERE client_uuid = ? ORDER BY updated_at DESC")
    .bind(auth.clientUuid).all<JsonObject>();
  return json({ channels: (rows.results || []).map((row) => ({
    ...row,
    provider_config: parseJsonObject<JsonObject>(String(row.provider_config_json || "{}"), {}),
    provider_config_json: undefined,
  })) });
}

async function saveMessagingChannel(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  const body = await request.json<JsonObject>();
  const connectionId = requiredSafeId(body.connection_id, "connection_id");
  const kind = String(body.kind || "").trim().toLowerCase();
  if (!new Set(["telegram", "whatsapp", "wechat_official", "we_com"]).has(kind)) throw new HttpError(400, "Unsupported messaging channel");
  const existing = await env.DB.prepare("SELECT * FROM messagingchannelconnectionrecord WHERE connection_id = ? AND client_uuid = ?").bind(connectionId, auth.clientUuid).first<MessagingChannelConnectionRow>();
  if (existing && existing.kind !== kind) throw new HttpError(409, "A connection kind cannot be changed after creation");
  const credentialRefInput = typeof body.credential_ref === "string" ? body.credential_ref.trim() : "";
  const webhookSecretRefInput = typeof body.webhook_secret_ref === "string" ? body.webhook_secret_ref.trim() : "";
  const verifyTokenRefInput = typeof body.verify_token_ref === "string" ? body.verify_token_ref.trim() : "";
  const credentialRef = requiredSafeId(credentialRefInput || existing?.credential_ref, "credential_ref");
  const webhookSecretRef = requiredSafeId(webhookSecretRefInput || existing?.webhook_secret_ref, "webhook_secret_ref");
  const verifyTokenRef = verifyTokenRefInput || existing?.verify_token_ref || "";
  if (!config.get(credentialRef, "")) throw new HttpError(422, "The configured credential_ref is unavailable");
  if (!config.get(webhookSecretRef, "")) throw new HttpError(422, "The configured webhook_secret_ref is unavailable");
  if (verifyTokenRef && !config.get(verifyTokenRef, "")) throw new HttpError(422, "The configured verify_token_ref is unavailable");
  const now = new Date().toISOString();
  const displayName = String(body.display_name || kind).trim().slice(0, 120);
  const accountId = String(body.account_id || "").trim().slice(0, 256);
  const providerConfig = body.provider_config && typeof body.provider_config === "object"
    ? JSON.stringify(body.provider_config)
    : existing?.provider_config_json || "{}";
  const enabled = body.enabled === true ? 1 : 0;
  await env.DB.prepare("INSERT INTO messagingchannelconnectionrecord (connection_id, client_uuid, kind, display_name, account_id, secret_ref, credential_ref, webhook_secret_ref, verify_token_ref, provider_config_json, enabled, state, created_at, updated_at) VALUES (?, ?, ?, ?, ?, '', ?, ?, ?, ?, ?, 'awaiting_verification', ?, ?) ON CONFLICT(connection_id) DO UPDATE SET display_name = excluded.display_name, account_id = excluded.account_id, credential_ref = excluded.credential_ref, webhook_secret_ref = excluded.webhook_secret_ref, verify_token_ref = excluded.verify_token_ref, provider_config_json = excluded.provider_config_json, enabled = excluded.enabled, updated_at = excluded.updated_at WHERE client_uuid = excluded.client_uuid")
    .bind(connectionId, auth.clientUuid, kind, displayName, accountId, credentialRef, webhookSecretRef, verifyTokenRef, providerConfig, enabled, now, now).run();
  let state = enabled === 1 ? "awaiting_verification" : "disabled";
  let lastError = "";
  if (kind === "telegram" && enabled === 1) {
    try {
      await configureTelegramWebhook(request, connectionId, config.get(credentialRef, ""), config.get(webhookSecretRef, ""), config);
      state = "connected";
    } catch (error) {
      state = "degraded";
      lastError = error instanceof Error ? error.message : String(error);
    }
  }
  await env.DB.prepare("UPDATE messagingchannelconnectionrecord SET state = ?, last_error = ? WHERE connection_id = ? AND client_uuid = ?")
    .bind(state, lastError.slice(0, 1000), connectionId, auth.clientUuid).run();
  return json({ connection_id: connectionId, kind, display_name: displayName, account_id: accountId, enabled: enabled === 1, state, last_error: lastError });
}

async function configureTelegramWebhook(request: Request, connectionId: string, botToken: string, webhookSecret: string, config: Config): Promise<void> {
  if (!botToken || !webhookSecret) throw new Error("Telegram credentials are unavailable");
  const origin = new URL(request.url).origin;
  const callback = `${origin}/v1/channels/webhook/telegram/${encodeURIComponent(connectionId)}`;
  const response = await fetch(`${providerApiBase(config, "TELEGRAM_API_BASE", "https://api.telegram.org")}/bot${botToken}/setWebhook`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify({ url: callback, secret_token: webhookSecret, allowed_updates: ["message"] }),
  });
  const payload: JsonObject = await response.json<JsonObject>().catch(() => ({} as JsonObject));
  if (!response.ok || payload.ok !== true) throw new Error(`Telegram setWebhook failed: ${String(payload.description || response.status)}`);
}

async function verifyWhatsAppWebhook(request: Request, env: Env, config: Config, connectionId: string): Promise<Response> {
  const connection = await loadMessagingConnection(env, connectionId, "whatsapp");
  const url = new URL(request.url);
  if (!connection.enabled) return new Response("Forbidden", { status: 403 });
  const expected = requireMessagingWebhookSecret(config, connection.verify_token_ref);
  if (url.searchParams.get("hub.mode") !== "subscribe" || !timingSafeEqual(url.searchParams.get("hub.verify_token") || "", expected)) return new Response("Forbidden", { status: 403 });
  return new Response(url.searchParams.get("hub.challenge") || "", { status: 200, headers: TEXT_HEADERS });
}

function requireMessagingWebhookSecret(config: Config, reference: string): string {
  const secret = reference.trim() ? config.get(reference, "") : "";
  if (!secret.trim()) throw new HttpError(403, "Messaging webhook credentials are unavailable");
  // Reject blank configuration without normalizing the bytes authenticated by the provider.
  return secret;
}

async function verifyWeChatOfficialWebhook(request: Request, env: Env, config: Config, connectionId: string): Promise<Response> {
  const connection = await loadMessagingConnection(env, connectionId, "wechat_official");
  if (!connection.enabled) return new Response("Not Found", { status: 404 });
  await verifyWeChatOfficialSignature(request, connection, config);
  const echo = new URL(request.url).searchParams.get("echostr") || "";
  if (!echo) throw new HttpError(400, "Missing WeChat echostr");
  return new Response(echo, { status: 200, headers: TEXT_HEADERS });
}

async function verifyWeChatOfficialSignature(request: Request, connection: MessagingChannelConnectionRow, config: Config): Promise<void> {
  const url = new URL(request.url);
  const timestamp = url.searchParams.get("timestamp") || "";
  const nonce = url.searchParams.get("nonce") || "";
  const supplied = url.searchParams.get("signature") || "";
  const token = requireMessagingWebhookSecret(config, connection.webhook_secret_ref);
  if (!timestamp || !nonce || !supplied) throw new HttpError(403, "Invalid WeChat callback credentials");
  const expected = await sha1Hex([token, timestamp, nonce].sort().join(""));
  if (!timingSafeEqual(supplied, expected)) throw new HttpError(403, "Invalid WeChat callback signature");
}

async function receiveMessagingWebhook(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const parts = path.split("/").filter(Boolean);
  const kind = parts[3] || "";
  const connectionId = parts[4] || "";
  const connection = await loadMessagingConnection(env, connectionId, kind);
  if (!connection.enabled) return new Response("Not Found", { status: 404 });
  const raw = await request.text();
  if (kind === "telegram") {
    const secret = requireMessagingWebhookSecret(config, connection.webhook_secret_ref);
    const supplied = request.headers.get("X-Telegram-Bot-Api-Secret-Token") || "";
    if (!timingSafeEqual(supplied, secret)) return new Response("Forbidden", { status: 403 });
  } else if (kind === "whatsapp") {
    const secret = requireMessagingWebhookSecret(config, connection.webhook_secret_ref);
    const supplied = (request.headers.get("X-Hub-Signature-256") || "").replace(/^sha256=/, "");
    const expected = hexBytes(await hmac(raw, secret));
    if (!timingSafeEqual(supplied, expected)) return new Response("Forbidden", { status: 403 });
  } else if (kind === "wechat_official") {
    await verifyWeChatOfficialSignature(request, connection, config);
    const envelope = weChatOfficialEnvelope(raw);
    const now = new Date().toISOString();
    const result = await env.DB.prepare("INSERT OR IGNORE INTO messagingchannelmessagerecord (connection_id, provider_event_id, external_conversation_id, external_sender_id, text, payload_json, received_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
      .bind(connectionId, envelope.providerEventId, envelope.conversationId, envelope.senderId, envelope.text, JSON.stringify(envelope.payload), envelope.receivedAt, now).run();
    await env.DB.prepare("UPDATE messagingchannelconnectionrecord SET state = 'connected', last_received_at = ?, last_error = NULL, updated_at = ? WHERE connection_id = ?")
      .bind(now, now, connectionId).run();
    return json({ accepted: true, inserted: (result.meta?.changes || 0) > 0 ? 1 : 0, duplicates: (result.meta?.changes || 0) > 0 ? 0 : 1 });
  } else if (kind === "we_com") {
    const encrypted = xmlElement(raw, "Encrypt");
    await verifyWeComSignature(request, connection, config, encrypted);
    const decrypted = await decryptWeComPayload(encrypted, connection, config);
    const envelope = weComEnvelope(decrypted);
    const now = new Date().toISOString();
    const result = await env.DB.prepare("INSERT OR IGNORE INTO messagingchannelmessagerecord (connection_id, provider_event_id, external_conversation_id, external_sender_id, text, payload_json, received_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
      .bind(connectionId, envelope.providerEventId, envelope.conversationId, envelope.senderId, envelope.text, JSON.stringify(envelope.payload), envelope.receivedAt, now).run();
    await env.DB.prepare("UPDATE messagingchannelconnectionrecord SET state = 'connected', last_received_at = ?, last_error = NULL, updated_at = ? WHERE connection_id = ?")
      .bind(now, now, connectionId).run();
    return new Response("success", { status: 200, headers: TEXT_HEADERS });
  } else {
    throw new HttpError(400, "Unsupported messaging channel webhook");
  }
  let payload: JsonObject;
  try { payload = JSON.parse(raw) as JsonObject; } catch { throw new HttpError(400, "Invalid webhook JSON"); }
  const envelopes = kind === "telegram" ? telegramEnvelopes(payload) : whatsAppEnvelopes(payload);
  const now = new Date().toISOString();
  let inserted = 0;
  for (const item of envelopes) {
    const result = await env.DB.prepare("INSERT OR IGNORE INTO messagingchannelmessagerecord (connection_id, provider_event_id, external_conversation_id, external_sender_id, text, payload_json, received_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
      .bind(connectionId, item.providerEventId, item.conversationId, item.senderId, item.text, JSON.stringify(item.payload), item.receivedAt, now).run();
    if ((result.meta?.changes || 0) > 0) inserted += 1;
  }
  await env.DB.prepare("UPDATE messagingchannelconnectionrecord SET state = 'connected', last_received_at = ?, last_error = NULL, updated_at = ? WHERE connection_id = ?")
    .bind(now, now, connectionId).run();
  return json({ accepted: true, inserted, duplicates: envelopes.length - inserted });
}

async function verifyWeComWebhook(request: Request, env: Env, config: Config, connectionId: string): Promise<Response> {
  const connection = await loadMessagingConnection(env, connectionId, "we_com");
  if (!connection.enabled) return new Response("Not Found", { status: 404 });
  const url = new URL(request.url);
  const encryptedEcho = url.searchParams.get("echostr") || "";
  if (!encryptedEcho) throw new HttpError(400, "Missing WeCom echostr");
  await verifyWeComSignature(request, connection, config, encryptedEcho);
  const echo = await decryptWeComPayload(encryptedEcho, connection, config);
  return new Response(echo, { status: 200, headers: TEXT_HEADERS });
}

async function verifyWeComSignature(request: Request, connection: MessagingChannelConnectionRow, config: Config, encrypted: string): Promise<void> {
  if (!encrypted) throw new HttpError(400, "Missing WeCom encrypted payload");
  const url = new URL(request.url);
  const timestamp = url.searchParams.get("timestamp") || "";
  const nonce = url.searchParams.get("nonce") || "";
  const supplied = url.searchParams.get("msg_signature") || "";
  const token = requireMessagingWebhookSecret(config, connection.verify_token_ref);
  if (!timestamp || !nonce || !supplied) throw new HttpError(403, "Invalid WeCom callback credentials");
  const expected = await sha1Hex([token, timestamp, nonce, encrypted].sort().join(""));
  if (!timingSafeEqual(supplied, expected)) throw new HttpError(403, "Invalid WeCom callback signature");
}

async function decryptWeComPayload(encrypted: string, connection: MessagingChannelConnectionRow, config: Config): Promise<string> {
  const encodingKey = requireMessagingWebhookSecret(config, connection.webhook_secret_ref).trim();
  if (!/^[A-Za-z0-9+/]{43}$/.test(encodingKey)) throw new HttpError(500, "Invalid WeCom EncodingAESKey configuration");
  const keyBytes = base64Bytes(`${encodingKey}=`);
  if (keyBytes.byteLength !== 32) throw new HttpError(500, "Invalid WeCom EncodingAESKey length");
  let cipherBytes: Uint8Array;
  try { cipherBytes = base64Bytes(encrypted); } catch { throw new HttpError(400, "Invalid WeCom encrypted payload"); }
  try {
    const key = await crypto.subtle.importKey("raw", keyBytes, { name: "AES-CBC" }, false, ["decrypt"]);
    const plaintext = await decryptWeComCiphertext(key, keyBytes, keyBytes.slice(0, 16), cipherBytes);
    if (plaintext.byteLength < 20) throw new Error("payload is too short");
    const view = new DataView(plaintext.buffer, plaintext.byteOffset, plaintext.byteLength);
    const messageLength = view.getUint32(16, false);
    const messageEnd = 20 + messageLength;
    if (messageLength <= 0 || messageEnd > plaintext.byteLength) throw new Error("payload length is invalid");
    const message = new TextDecoder().decode(plaintext.slice(20, messageEnd));
    const receiveId = new TextDecoder().decode(plaintext.slice(messageEnd));
    const providerConfig = parseProviderConfig(connection.provider_config_json);
    const expectedReceiveId = String(providerConfig.corp_id || connection.account_id || "").trim();
    if (expectedReceiveId && receiveId !== expectedReceiveId) throw new HttpError(403, "WeCom callback receive id mismatch");
    return message;
  } catch (error) {
    if (error instanceof HttpError) throw error;
    throw new HttpError(400, `Unable to decrypt WeCom callback: ${error instanceof Error ? error.message : String(error)}`);
  }
}

async function decryptWeComCiphertext(key: CryptoKey, rawKey: Uint8Array, iv: Uint8Array, cipherBytes: Uint8Array): Promise<Uint8Array> {
  try {
    return new Uint8Array(await crypto.subtle.decrypt({ name: "AES-CBC", iv }, key, cipherBytes));
  } catch (originalError) {
    // WeCom uses PKCS#7 with a 32-byte block size, while WebCrypto validates
    // AES-CBC padding against AES's 16-byte block. Legal WeCom padding of
    // 17...32 bytes is therefore rejected. Append a synthetic AES block whose
    // plaintext is standard 0x10 padding, let WebCrypto remove that block, then
    // validate and remove WeCom's original padding ourselves.
    if (cipherBytes.byteLength < 16 || cipherBytes.byteLength % 16 !== 0) throw originalError;
    const lastCipherBlock = cipherBytes.slice(cipherBytes.byteLength - 16);
    const encryptionKey = await crypto.subtle.importKey("raw", rawKey, { name: "AES-CBC" }, false, ["encrypt"]);
    const paddedBlock = new Uint8Array(16).fill(16);
    const encrypted = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-CBC", iv: lastCipherBlock }, encryptionKey, paddedBlock));
    const extended = new Uint8Array(cipherBytes.byteLength + 16);
    extended.set(cipherBytes);
    extended.set(encrypted.slice(0, 16), cipherBytes.byteLength);
    const decrypted = new Uint8Array(await crypto.subtle.decrypt({ name: "AES-CBC", iv }, key, extended));
    const padding = decrypted.at(-1) || 0;
    if (padding < 17 || padding > 32 || decrypted.byteLength < padding) throw originalError;
    for (let index = decrypted.byteLength - padding; index < decrypted.byteLength; index += 1) {
      if (decrypted[index] !== padding) throw originalError;
    }
    return decrypted.slice(0, decrypted.byteLength - padding);
  }
}

function weComEnvelope(xml: string) {
  const messageType = xmlElement(xml, "MsgType");
  if (messageType !== "text") throw new HttpError(422, `Unsupported WeCom message type: ${messageType || "unknown"}`);
  const sender = xmlElement(xml, "FromUserName");
  const text = xmlElement(xml, "Content");
  const createdAt = xmlElement(xml, "CreateTime");
  const messageId = xmlElement(xml, "MsgId") || `${sender}:${createdAt}:${text}`;
  if (!sender || !text) throw new HttpError(400, "Incomplete WeCom text message");
  return {
    providerEventId: messageId,
    conversationId: sender,
    senderId: sender,
    text,
    receivedAt: providerTimestamp(createdAt),
    payload: { msg_type: messageType, from_user: sender, create_time: createdAt, msg_id: messageId },
  };
}

function xmlElement(xml: string, name: string): string {
  const match = xml.match(new RegExp(`<${name}(?:\\s[^>]*)?>(?:<!\\[CDATA\\[([\\s\\S]*?)\\]\\]>|([\\s\\S]*?))<\\/${name}>`, "i"));
  return decodeXmlEntities((match?.[1] ?? match?.[2] ?? "").trim());
}

function decodeXmlEntities(value: string): string {
  return value.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&amp;/g, "&");
}

function base64Bytes(value: string): Uint8Array {
  const binary = atob(value);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

function parseProviderConfig(value: string): JsonObject {
  try {
    const parsed = JSON.parse(value || "{}");
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed as JsonObject : {};
  } catch {
    return {};
  }
}

async function sha1Hex(value: string): Promise<string> {
  return hexBytes(await crypto.subtle.digest("SHA-1", new TextEncoder().encode(value)));
}

async function loadMessagingConnection(env: Env, connectionId: string, kind: string): Promise<MessagingChannelConnectionRow> {
  const safeId = requiredSafeId(connectionId, "connection_id");
  const row = await env.DB.prepare("SELECT * FROM messagingchannelconnectionrecord WHERE connection_id = ? AND kind = ?").bind(safeId, kind).first<MessagingChannelConnectionRow>();
  if (!row) throw new HttpError(404, "Messaging channel not found");
  return row;
}

async function claimMessagingChannelMessage(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  const body: JsonObject = await request.json<JsonObject>().catch(() => ({} as JsonObject));
  const requestedLeaseSeconds = Number(body.lease_seconds ?? 180);
  const leaseSeconds = Number.isFinite(requestedLeaseSeconds)
    ? Math.min(900, Math.max(30, Math.floor(requestedLeaseSeconds)))
    : 180;
  const now = new Date();
  const expiresAt = new Date(now.getTime() + leaseSeconds * 1000).toISOString();
  const candidates = await env.DB.prepare(
    "SELECT m.id FROM messagingchannelmessagerecord m JOIN messagingchannelconnectionrecord c ON c.connection_id = m.connection_id WHERE c.client_uuid = ? AND c.enabled = 1 AND (m.status = 'received' OR (m.status = 'processing' AND m.lease_expires_at < ?)) ORDER BY m.id LIMIT 10"
  ).bind(auth.clientUuid, now.toISOString()).all<{ id: number }>();
  for (const candidate of candidates.results || []) {
    const leaseId = crypto.randomUUID();
    const claimed = await env.DB.prepare(
      "UPDATE messagingchannelmessagerecord SET status = 'processing', lease_id = ?, lease_expires_at = ?, attempt_count = attempt_count + 1, last_error = NULL WHERE id = ? AND (status = 'received' OR (status = 'processing' AND lease_expires_at < ?))"
    ).bind(leaseId, expiresAt, candidate.id, now.toISOString()).run();
    if ((claimed.meta?.changes || 0) === 0) continue;
    const row = await env.DB.prepare(
      "SELECT m.id, m.connection_id, c.kind, m.provider_event_id, m.external_conversation_id, m.external_sender_id, m.text, m.received_at, m.attempt_count FROM messagingchannelmessagerecord m JOIN messagingchannelconnectionrecord c ON c.connection_id = m.connection_id WHERE m.id = ? AND c.client_uuid = ?"
    ).bind(candidate.id, auth.clientUuid).first<JsonObject>();
    if (!row) return json({ message: null });
    return json({
      message: {
        schema_version: 1,
        message_id: Number(row.id),
        lease_id: leaseId,
        lease_expires_at: expiresAt,
        attempt_count: Number(row.attempt_count || 1),
        envelope: {
          schema_version: 1,
          connection_id: String(row.connection_id || ""),
          kind: String(row.kind || ""),
          provider_event_id: String(row.provider_event_id || ""),
          external_conversation_id: String(row.external_conversation_id || ""),
          external_sender_id: String(row.external_sender_id || ""),
          received_at: String(row.received_at || ""),
          text: String(row.text || ""),
          attachment_ids: [],
        },
      },
    });
  }
  return json({ message: null });
}

async function completeMessagingChannelMessage(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const auth = await requireAuth(request, config);
  const id = Number(path.split("/").at(-2));
  if (!Number.isSafeInteger(id) || id <= 0) throw new HttpError(400, "Invalid message id");
  const body = await request.json<JsonObject>();
  if (body.schema_version !== undefined && Number(body.schema_version) !== 1) throw new HttpError(400, "Unsupported messaging channel schema_version");
  const leaseId = requiredSafeId(body.lease_id, "lease_id");
  const replyText = String(body.reply_text || "").trim();
  if (!replyText || replyText.length > 40_000) throw new HttpError(400, "Invalid reply_text");
  const nolaConversationId = typeof body.nola_conversation_id === "string"
    ? body.nola_conversation_id.trim().slice(0, 256)
    : "";
  const row = await env.DB.prepare(
    "SELECT m.*, c.client_uuid, c.kind, c.account_id, c.credential_ref, c.provider_config_json FROM messagingchannelmessagerecord m JOIN messagingchannelconnectionrecord c ON c.connection_id = m.connection_id WHERE m.id = ? AND c.client_uuid = ?"
  ).bind(id, auth.clientUuid).first<JsonObject>();
  if (!row) throw new HttpError(404, "Messaging channel message not found");
  if (row.status !== "processing" || row.lease_id !== leaseId || String(row.lease_expires_at || "") < new Date().toISOString()) {
    throw new HttpError(409, "Message lease is no longer valid");
  }
  try {
    const providerReplyId = await deliverMessagingReply(row, replyText, config);
    const completedAt = new Date().toISOString();
    await env.DB.prepare(
      "UPDATE messagingchannelmessagerecord SET status = 'sent', reply_text = ?, provider_reply_id = ?, nola_conversation_id = CASE WHEN ? = '' THEN nola_conversation_id ELSE ? END, processed_at = ?, lease_id = NULL, lease_expires_at = NULL, last_error = NULL WHERE id = ? AND lease_id = ?"
    ).bind(replyText, providerReplyId, nolaConversationId, nolaConversationId, completedAt, id, leaseId).run();
    return json({ accepted: true, status: "sent", provider_reply_id: providerReplyId });
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    const attempts = Number(row.attempt_count || 1);
    const nextStatus = attempts >= 5 ? "failed" : "received";
    await env.DB.prepare(
      "UPDATE messagingchannelmessagerecord SET status = ?, last_error = ?, lease_id = NULL, lease_expires_at = NULL WHERE id = ? AND lease_id = ?"
    ).bind(nextStatus, detail.slice(0, 1000), id, leaseId).run();
    throw new HttpError(502, `Channel delivery failed: ${detail}`);
  }
}

async function renewMessagingChannelMessage(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const auth = await requireAuth(request, config);
  const id = Number(path.split("/").at(-2));
  if (!Number.isSafeInteger(id) || id <= 0) throw new HttpError(400, "Invalid message id");
  const body = await request.json<JsonObject>();
  if (body.schema_version !== undefined && Number(body.schema_version) !== 1) throw new HttpError(400, "Unsupported messaging channel schema_version");
  const leaseId = requiredSafeId(body.lease_id, "lease_id");
  const requestedLeaseSeconds = Number(body.lease_seconds ?? 180);
  const leaseSeconds = Number.isFinite(requestedLeaseSeconds)
    ? Math.min(900, Math.max(30, Math.floor(requestedLeaseSeconds)))
    : 180;
  const leaseExpiresAt = new Date(Date.now() + leaseSeconds * 1000).toISOString();
  const result = await env.DB.prepare(
    "UPDATE messagingchannelmessagerecord SET lease_expires_at = ? WHERE id = ? AND connection_id IN (SELECT connection_id FROM messagingchannelconnectionrecord WHERE client_uuid = ?) AND status = 'processing' AND lease_id = ? AND lease_expires_at >= ?"
  ).bind(leaseExpiresAt, id, auth.clientUuid, leaseId, new Date().toISOString()).run();
  if ((result.meta?.changes || 0) === 0) throw new HttpError(409, "Message lease is no longer valid");
  return json({ accepted: true, lease_expires_at: leaseExpiresAt });
}

async function progressMessagingChannelMessage(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const auth = await requireAuth(request, config);
  const id = Number(path.split("/").at(-2));
  if (!Number.isSafeInteger(id) || id <= 0) throw new HttpError(400, "Invalid message id");
  const body = await request.json<JsonObject>();
  if (body.schema_version !== undefined && Number(body.schema_version) !== 1) throw new HttpError(400, "Unsupported messaging channel schema_version");
  const leaseId = requiredSafeId(body.lease_id, "lease_id");
  const text = String(body.text || "").slice(0, 4096);
  const row = await env.DB.prepare(
    "SELECT m.*, c.client_uuid, c.kind, c.credential_ref FROM messagingchannelmessagerecord m JOIN messagingchannelconnectionrecord c ON c.connection_id = m.connection_id WHERE m.id = ? AND c.client_uuid = ?"
  ).bind(id, auth.clientUuid).first<JsonObject>();
  if (!row) throw new HttpError(404, "Messaging channel message not found");
  if (row.status !== "processing" || row.lease_id !== leaseId || String(row.lease_expires_at || "") < new Date().toISOString()) {
    throw new HttpError(409, "Message lease is no longer valid");
  }
  if (String(row.kind || "") !== "telegram") {
    return json({ accepted: true, delivered: false, reason: "channel_does_not_support_drafts" });
  }
  const credential = config.get(String(row.credential_ref || ""), "");
  if (!credential) throw new HttpError(422, "Channel credential is unavailable");
  const chatId = Number(row.external_conversation_id);
  if (!Number.isSafeInteger(chatId)) return json({ accepted: true, delivered: false, reason: "drafts_require_private_numeric_chat" });
  const response = await fetch(`${providerApiBase(config, "TELEGRAM_API_BASE", "https://api.telegram.org")}/bot${credential}/sendMessageDraft`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify({ chat_id: chatId, draft_id: id, text }),
  });
  const payload: JsonObject = await response.json<JsonObject>().catch(() => ({} as JsonObject));
  if (!response.ok || payload.ok !== true) throw new HttpError(502, `Telegram ${response.status}: ${String(payload.description || "sendMessageDraft failed")}`);
  return json({ accepted: true, delivered: true });
}

async function failMessagingChannelMessage(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const auth = await requireAuth(request, config);
  const id = Number(path.split("/").at(-2));
  if (!Number.isSafeInteger(id) || id <= 0) throw new HttpError(400, "Invalid message id");
  const body = await request.json<JsonObject>();
  if (body.schema_version !== undefined && Number(body.schema_version) !== 1) throw new HttpError(400, "Unsupported messaging channel schema_version");
  const leaseId = requiredSafeId(body.lease_id, "lease_id");
  const detail = String(body.error || "Host execution failed").trim().slice(0, 1000);
  const retryable = body.retryable !== false;
  const row = await env.DB.prepare(
    "SELECT m.attempt_count FROM messagingchannelmessagerecord m JOIN messagingchannelconnectionrecord c ON c.connection_id = m.connection_id WHERE m.id = ? AND c.client_uuid = ? AND m.status = 'processing' AND m.lease_id = ?"
  ).bind(id, auth.clientUuid, leaseId).first<JsonObject>();
  if (!row) throw new HttpError(409, "Message lease is no longer valid");
  const nextStatus = retryable && Number(row.attempt_count || 1) < 5 ? "received" : "failed";
  const result = await env.DB.prepare(
    "UPDATE messagingchannelmessagerecord SET status = ?, last_error = ?, processed_at = CASE WHEN ? = 'failed' THEN ? ELSE processed_at END, lease_id = NULL, lease_expires_at = NULL WHERE id = ? AND status = 'processing' AND lease_id = ?"
  ).bind(nextStatus, detail || "Host execution failed", nextStatus, new Date().toISOString(), id, leaseId).run();
  if ((result.meta?.changes || 0) === 0) throw new HttpError(409, "Message lease is no longer valid");
  return json({ accepted: true, status: nextStatus });
}

async function deliverMessagingReply(row: JsonObject, text: string, config: Config): Promise<string> {
  const kind = String(row.kind || "");
  const credential = config.get(String(row.credential_ref || ""), "");
  if (!credential) throw new Error("Channel credential is unavailable");
  if (kind === "telegram") {
    const response = await fetch(`${providerApiBase(config, "TELEGRAM_API_BASE", "https://api.telegram.org")}/bot${credential}/sendMessage`, {
      method: "POST",
      headers: JSON_HEADERS,
      body: JSON.stringify({ chat_id: String(row.external_conversation_id), text }),
    });
    const payload: JsonObject = await response.json<JsonObject>().catch(() => ({} as JsonObject));
    if (!response.ok || payload.ok !== true) throw new Error(`Telegram ${response.status}: ${String(payload.description || "sendMessage failed")}`);
    return String((payload.result as JsonObject | undefined)?.message_id || "");
  }
  if (kind === "whatsapp") {
    const providerConfig = parseJsonObject<JsonObject>(String(row.provider_config_json || "{}"), {});
    const version = String(providerConfig.graph_version || "v23.0");
    const phoneNumberId = String(providerConfig.phone_number_id || row.account_id || "");
    if (!phoneNumberId) throw new Error("WhatsApp phone_number_id is unavailable");
    const response = await fetch(`${providerApiBase(config, "WHATSAPP_GRAPH_API_BASE", "https://graph.facebook.com")}/${version}/${encodeURIComponent(phoneNumberId)}/messages`, {
      method: "POST",
      headers: { ...JSON_HEADERS, authorization: `Bearer ${credential}` },
      body: JSON.stringify({ messaging_product: "whatsapp", to: String(row.external_sender_id), type: "text", text: { body: text, preview_url: false } }),
    });
    const payload: JsonObject = await response.json<JsonObject>().catch(() => ({} as JsonObject));
    if (!response.ok) throw new Error(`WhatsApp ${response.status}: ${JSON.stringify(payload).slice(0, 500)}`);
    const messages = Array.isArray(payload.messages) ? payload.messages : [];
    return String(messages[0]?.id || "");
  }
  if (kind === "we_com") {
    const providerConfig = parseJsonObject<JsonObject>(String(row.provider_config_json || "{}"), {});
    const corpId = String(providerConfig.corp_id || "");
    const agentId = String(providerConfig.agent_id || row.account_id || "");
    if (!corpId || !agentId) throw new Error("WeCom corp_id or agent_id is unavailable");
    const apiBase = providerApiBase(config, "WECOM_API_BASE", "https://qyapi.weixin.qq.com");
    const tokenResponse = await fetch(`${apiBase}/cgi-bin/gettoken?corpid=${encodeURIComponent(corpId)}&corpsecret=${encodeURIComponent(credential)}`);
    const tokenPayload: JsonObject = await tokenResponse.json<JsonObject>().catch(() => ({} as JsonObject));
    if (!tokenResponse.ok || Number(tokenPayload.errcode || 0) !== 0) throw new Error(`WeCom gettoken failed: ${JSON.stringify(tokenPayload).slice(0, 500)}`);
    const response = await fetch(`${apiBase}/cgi-bin/message/send?access_token=${encodeURIComponent(String(tokenPayload.access_token || ""))}`, {
      method: "POST",
      headers: JSON_HEADERS,
      body: JSON.stringify({ touser: String(row.external_sender_id), msgtype: "text", agentid: Number(agentId), text: { content: text }, safe: 0 }),
    });
    const payload: JsonObject = await response.json<JsonObject>().catch(() => ({} as JsonObject));
    if (!response.ok || Number(payload.errcode || 0) !== 0) throw new Error(`WeCom send failed: ${JSON.stringify(payload).slice(0, 500)}`);
    return String(payload.msgid || "");
  }
  if (kind === "wechat_official") {
    const providerConfig = parseJsonObject<JsonObject>(String(row.provider_config_json || "{}"), {});
    const appId = String(providerConfig.app_id || row.account_id || "");
    if (!appId) throw new Error("WeChat Official Account app_id is unavailable");
    const apiBase = providerApiBase(config, "WECHAT_API_BASE", "https://api.weixin.qq.com");
    const tokenResponse = await fetch(`${apiBase}/cgi-bin/token?grant_type=client_credential&appid=${encodeURIComponent(appId)}&secret=${encodeURIComponent(credential)}`);
    const tokenPayload: JsonObject = await tokenResponse.json<JsonObject>().catch(() => ({} as JsonObject));
    if (!tokenResponse.ok || !tokenPayload.access_token) throw new Error(`WeChat access token failed: ${JSON.stringify(tokenPayload).slice(0, 500)}`);
    const response = await fetch(`${apiBase}/cgi-bin/message/custom/send?access_token=${encodeURIComponent(String(tokenPayload.access_token))}`, {
      method: "POST",
      headers: JSON_HEADERS,
      body: JSON.stringify({ touser: String(row.external_sender_id), msgtype: "text", text: { content: text } }),
    });
    const payload: JsonObject = await response.json<JsonObject>().catch(() => ({} as JsonObject));
    if (!response.ok || Number(payload.errcode || 0) !== 0) throw new Error(`WeChat send failed: ${JSON.stringify(payload).slice(0, 500)}`);
    return String(payload.msgid || "");
  }
  throw new Error(`Unsupported messaging channel: ${kind}`);
}

function providerApiBase(config: Config, key: string, productionDefault: string): string {
  const configured = config.get(key, productionDefault).trim().replace(/\/+$/, "");
  let parsed: URL;
  try {
    parsed = new URL(configured);
  } catch {
    throw new Error(`${key} is not a valid URL`);
  }
  if (!new Set(["http:", "https:"]).has(parsed.protocol)) throw new Error(`${key} must use HTTP or HTTPS`);
  return configured;
}

function telegramEnvelopes(payload: JsonObject) {
  const updateId = String(payload.update_id ?? "");
  const message = (payload.message || payload.edited_message) as JsonObject | undefined;
  const chat = message?.chat as JsonObject | undefined;
  const from = message?.from as JsonObject | undefined;
  if (!updateId || !message || chat?.id === undefined || from?.id === undefined) return [];
  return [{ providerEventId: updateId, conversationId: String(chat.id), senderId: String(from.id), text: String(message.text || message.caption || ""), receivedAt: providerTimestamp(message.date), payload }];
}

function whatsAppEnvelopes(payload: JsonObject) {
  const result: Array<{ providerEventId: string; conversationId: string; senderId: string; text: string; receivedAt: string; payload: JsonObject }> = [];
  const entries = Array.isArray(payload.entry) ? payload.entry : [];
  for (const entry of entries) for (const change of (Array.isArray(entry?.changes) ? entry.changes : [])) {
    const messages = Array.isArray(change?.value?.messages) ? change.value.messages : [];
    for (const message of messages) if (message?.id && message?.from) result.push({ providerEventId: String(message.id), conversationId: String(message.from), senderId: String(message.from), text: String(message.text?.body || message.image?.caption || message.document?.caption || ""), receivedAt: providerTimestamp(message.timestamp), payload: message as JsonObject });
  }
  return result;
}

function weChatOfficialEnvelope(xml: string) {
  const sender = xmlElement(xml, "FromUserName");
  const recipient = xmlElement(xml, "ToUserName");
  const text = xmlElement(xml, "Content");
  const messageId = xmlElement(xml, "MsgId");
  const createdAt = xmlElement(xml, "CreateTime");
  if (!sender || !recipient || !messageId) throw new HttpError(400, "Invalid WeChat Official Account message");
  return {
    providerEventId: messageId,
    conversationId: sender,
    senderId: sender,
    text,
    receivedAt: providerTimestamp(createdAt),
    payload: { to_user_name: recipient, from_user_name: sender, message_id: messageId, message_type: xmlElement(xml, "MsgType") },
  };
}

function providerTimestamp(value: unknown): string {
  const seconds = Number(value || 0);
  return Number.isFinite(seconds) && seconds > 0 ? new Date(seconds * 1000).toISOString() : new Date().toISOString();
}

function requiredSafeId(value: unknown, name: string): string {
  const text = typeof value === "string" ? value.trim() : "";
  if (!/^[A-Za-z0-9_-]{3,128}$/.test(text)) throw new HttpError(400, `Invalid ${name}`);
  return text;
}

function hexBytes(value: ArrayBuffer): string {
  return [...new Uint8Array(value)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function handleRequest(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  const url = new URL(request.url);
  const path = url.pathname.replace(/\/+$/, "") || "/";
  const config = loadConfig(env);
  const guarded = scanGuardResponse(request, path, config);
  if (guarded) return guarded;

  if (request.method === "OPTIONS") return cors(new Response(null, { status: 204 }));
  if (request.method === "GET" && path === "/healthz") return health(config);
  if (request.method === "GET" && path === "/v1/channels") return listMessagingChannels(request, env, config);
  if (request.method === "PUT" && path === "/v1/channels") return saveMessagingChannel(request, env, config);
  if (request.method === "POST" && path === "/v1/channels/inbox/claim") return claimMessagingChannelMessage(request, env, config);
  if (request.method === "POST" && /^\/v1\/channels\/inbox\/\d+\/complete$/.test(path)) return completeMessagingChannelMessage(request, env, config, path);
  if (request.method === "POST" && /^\/v1\/channels\/inbox\/\d+\/renew$/.test(path)) return renewMessagingChannelMessage(request, env, config, path);
  if (request.method === "POST" && /^\/v1\/channels\/inbox\/\d+\/progress$/.test(path)) return progressMessagingChannelMessage(request, env, config, path);
  if (request.method === "POST" && /^\/v1\/channels\/inbox\/\d+\/fail$/.test(path)) return failMessagingChannelMessage(request, env, config, path);
  if (request.method === "GET" && path.startsWith("/v1/channels/webhook/whatsapp/")) return verifyWhatsAppWebhook(request, env, config, path.split("/").pop() || "");
  if (request.method === "GET" && path.startsWith("/v1/channels/webhook/wechat_official/")) return verifyWeChatOfficialWebhook(request, env, config, path.split("/").pop() || "");
  if (request.method === "GET" && path.startsWith("/v1/channels/webhook/we_com/")) return verifyWeComWebhook(request, env, config, path.split("/").pop() || "");
  if (request.method === "POST" && path.startsWith("/v1/channels/webhook/")) return receiveMessagingWebhook(request, env, config, path);
  if (request.method === "GET" && path === "/admin/outbox/dlq/count") return adminDlqCount(request, config);
  if (request.method === "POST" && path === "/admin/outbox/dlq/replay") return adminDlqReplay(request, config, url);

  if (request.method === "GET" && path === "/search/serper") return serperSearch(url, config);

  if (request.method === "POST" && path === "/v1/auth/register") return authRegister(request, env, config);
  if (request.method === "POST" && path === "/v1/auth/login") return authLogin(request, env, config);
  if (request.method === "GET" && path === "/v1/auth/me") return authMe(request, env, config);
  if (request.method === "POST" && path === "/api/mobile/login") return mobileLogin(request, env, config);
  if (request.method === "GET" && path === "/api/mobile/me") return mobileMe(request, env, config);
  if (request.method === "GET" && path === "/api/mobile/agent/settings") return mobileAgentSettings(request, env, config);
  if (request.method === "POST" && path === "/api/mobile/attachments/parse") return mobileAttachmentParse(request, config);

  if (request.method === "GET" && path === "/v1/agent/settings") return agentSettingsGet(request, env, config);
  if (request.method === "POST" && path === "/v1/agent/settings") return agentSettingsPost(request, env, config);
  if (request.method === "GET" && path === "/v1/agent/permission_rules") return permissionRulesGet(request, env, config);
  if (request.method === "POST" && path === "/v1/agent/permission_rules") return permissionRulesPost(request, env, config);
  if (request.method === "DELETE" && path.startsWith("/v1/agent/permission_rules/")) return permissionRulesDelete(request, env, config, path);
  if (request.method === "GET" && path === "/v1/agent/tools") return agentToolsList();

  if (request.method === "GET" && path === "/v1/models") return listModels(request, env, config);
  if (request.method === "POST" && path === "/v1/chat/completions") return chatCompletions(request, env, config);

  if (request.method === "GET" && path === "/v1/chat/conversations") return listChatConversations(request, env, config, url);
  if (request.method === "POST" && path === "/v1/chat/conversations") return createChatConversation(request, env, config);
  if (request.method === "GET" && path.includes("/v1/chat/conversations/") && path.endsWith("/messages")) return listChatMessages(request, env, config, path);
  if (request.method === "POST" && path.includes("/v1/chat/conversations/") && path.endsWith("/messages")) return addChatMessage(request, env, config, path);
  if (request.method === "PATCH" && path.startsWith("/v1/chat/conversations/")) return updateChatConversation(request, env, config, path);
  if (request.method === "DELETE" && path.startsWith("/v1/chat/conversations/")) return deleteChatConversation(request, env, config, path);

  // Canonical clients own product state. The backend only stores an append-only,
  // revisioned replica log and never manufactures or rewrites product facts.
  if (request.method === "POST" && path === "/v1/sync/replicas") return pushCanonicalReplicas(request, env, config);
  if (request.method === "GET" && path === "/v1/sync/replicas") return pullCanonicalReplicas(request, env, config, url);

  if (request.method === "POST" && path === "/v1/devices/register") return registerDevice(request, env, config);
  if (request.method === "POST" && path === "/v1/sources/sync") return syncSources(request, env, config);
  if (request.method === "GET" && path === "/v1/sources") return listSources(request, env, config, url);
  if (request.method === "DELETE" && path.startsWith("/v1/sources/")) return deleteSource(request, env, config, path);
  if (request.method === "POST" && path.startsWith("/v1/sources/") && path.endsWith("/run_now")) return runSourceNow(request, env, config, path, ctx);
  if (request.method === "GET" && path === "/v1/results") return listResults(request, env, config, url);
  if (request.method === "GET" && path === "/v1/notifications") return listNotifications(request, env, config, url);
  if (request.method === "GET" && path === "/v1/notifications/unread_count") return notificationUnreadCount(request, env, config);
  if (request.method === "POST" && path === "/v1/notifications/read_all") return markAllNotificationsRead(request, env, config);
  if (request.method === "POST" && path.startsWith("/v1/notifications/") && path.endsWith("/read")) return markNotificationRead(request, env, config, path);
  if (request.method === "GET" && path.startsWith("/v1/results/")) return getResult(request, env, config, path);
  if (request.method === "POST" && path.startsWith("/v1/results/") && path.endsWith("/ack")) return ackResult(request, env, config, path);
  if (request.method === "POST" && path === "/v1/debug/push/silent") return debugPushSilent(request, config);
  if (request.method === "POST" && path === "/v1/debug/push/alert") return debugPushAlert(request, config);
  if (request.method === "POST" && path === "/v1/debug/apns/validate_device") return debugApnsValidateDevice(request, config);
  if (request.method === "GET" && path === "/v1/debug/apns/status") return debugApnsStatus(request, config);
  if (request.method === "GET" && path.startsWith("/v1/debug/push/client/")) return debugPushClient(request, env, config, path.split("/").pop() || "");

  return json({ detail: "Not found" }, 404);
}

function scanGuardClientIp(request: Request): string {
  const xff = (request.headers.get("x-forwarded-for") || "").trim();
  if (xff) {
    const first = xff.split(",")[0]?.trim();
    if (first) return first;
  }
  const realIp = (request.headers.get("x-real-ip") || "").trim();
  if (realIp) return realIp;
  return "unknown";
}

function isKnownApiPath(path: string): boolean {
  return SCAN_ALLOWED_PREFIXES.some((prefix) => path === prefix || (prefix !== "/" && path.startsWith(`${prefix}/`)));
}

function isSuspiciousScanPath(path: string): boolean {
  const lower = (path || "").toLowerCase();
  return SCAN_SUSPICIOUS_PATH_MARKERS.some((marker) => lower.includes(marker));
}

function scanGuardResponse(request: Request, path: string, config: Config): Response | null {
  if (!config.bool("SCAN_GUARD_ENABLED", true)) return null;

  const method = (request.method || "").toUpperCase();
  if (!ALLOWED_HTTP_METHODS.has(method)) {
    return json({ detail: "Method not allowed" }, 405);
  }
  if (isKnownApiPath(path)) return null;

  const now = Date.now();
  const cutoff = now - 60_000;
  const ip = scanGuardClientIp(request);
  const limit = Math.max(10, config.int("SCAN_GUARD_UNKNOWN_PATH_LIMIT_PER_MIN", 60));
  const banMs = Math.max(30, config.int("SCAN_GUARD_BAN_SECONDS", 300)) * 1000;

  for (const [bannedIp, until] of scanBanUntilByIp.entries()) {
    if (until <= now) {
      scanBanUntilByIp.delete(bannedIp);
      scanEventsByIp.delete(bannedIp);
    }
  }

  const bannedUntil = scanBanUntilByIp.get(ip) || 0;
  if (bannedUntil > now) {
    return json({ detail: "Too many unknown-path requests" }, 429);
  }

  const events = (scanEventsByIp.get(ip) || []).filter((ts) => ts >= cutoff);
  const weight = isSuspiciousScanPath(path) ? 3 : 1;
  for (let i = 0; i < weight; i += 1) events.push(now);
  scanEventsByIp.set(ip, events);

  if (events.length > limit) {
    scanBanUntilByIp.set(ip, now + banMs);
    scanEventsByIp.delete(ip);
    return json({ detail: "Too many unknown-path requests" }, 429);
  }
  return json({ detail: "Not found" }, 404);
}

function loadConfig(env: Env): Config {
  const raw: JsonObject = {};
  const source = env.nola_backend_env ?? env.NOLA_BACKEND_ENV;
  if (typeof source === "string" && source.trim()) {
    Object.assign(raw, JSON.parse(source));
  } else if (source && typeof source === "object") {
    Object.assign(raw, source);
  }
  for (const [key, value] of Object.entries(env)) {
    if (key === "DB" || key === "nola_backend_env" || key === "NOLA_BACKEND_ENV") continue;
    if (raw[key] === undefined) raw[key] = value;
  }
  const toString = (value: unknown): string => {
    if (value === undefined || value === null) return "";
    if (typeof value === "string") return value;
    if (typeof value === "boolean") return value ? "true" : "false";
    if (typeof value === "number") return String(value);
    return JSON.stringify(value);
  };
  return {
    raw,
    get(name, fallback = "") {
      const value = toString(raw[name]);
      return value || fallback;
    },
    bool(name, fallback = false) {
      const value = toString(raw[name]).toLowerCase();
      if (!value) return fallback;
      return ["1", "true", "yes", "on"].includes(value);
    },
    int(name, fallback) {
      const text = toString(raw[name]).trim();
      if (!text) return fallback;
      const value = Number(text);
      return Number.isFinite(value) ? value : fallback;
    },
  };
}

function health(config: Config): Response {
  return json({
    ok: true,
    runtime: "cloudflare-worker",
    database: "d1",
    time: new Date().toISOString(),
    buildId: config.get("GATEWAY_BUILD_ID", "nola-worker"),
    pushFeatureEnabled: config.bool("PUSH_FEATURE_ENABLED", false),
    pushSchedulerEnabled: false,
  });
}

function requireAdmin(request: Request, config: Config): void {
  const expected = config.get("ADMIN_API_KEY");
  const actual = (request.headers.get("x-admin-token") || request.headers.get("X-Admin-Token") || "").trim();
  if (!expected || actual !== expected) throw new HttpError(401, "Invalid admin token");
}

function requireDebugAdmin(request: Request, config: Config): void {
  const expected = config.get("DEBUG_ADMIN_TOKEN");
  const actual = (request.headers.get("x-debug-token") || request.headers.get("X-Debug-Token") || "").trim();
  if (!expected || actual !== expected) throw new HttpError(401, "Invalid debug token");
}

function adminDlqCount(request: Request, config: Config): Response {
  requireAdmin(request, config);
  return json({ count: 0, mode: "worker-d1", note: "outbox dlq is not used in worker mode" });
}

function adminDlqReplay(request: Request, config: Config, url: URL): Response {
  requireAdmin(request, config);
  const limit = Number(url.searchParams.get("limit") || 100);
  return json({ replayed: 0, limit, mode: "worker-d1", note: "outbox dlq replay is not used in worker mode" });
}

async function serperSearch(url: URL, config: Config): Promise<Response> {
  const apiKey = config.get("SERPER_API_KEY");
  if (!apiKey) throw new HttpError(500, "SERPER_API_KEY not configured");
  const q = url.searchParams.get("q") || "";
  if (!q.trim()) throw new HttpError(400, "q is required");
  const gl = url.searchParams.get("gl") || "us";
  const hl = url.searchParams.get("hl") || "en";
  const page = Number(url.searchParams.get("page") || 1);
  const endpoint = config.get("SERPER_API_URL", "https://google.serper.dev/search");
  const resp = await fetch(endpoint, {
    method: "POST",
    headers: { "X-API-KEY": apiKey, "Content-Type": "application/json" },
    body: JSON.stringify({ q, gl, hl, page }),
  });
  const text = await resp.text();
  if (!resp.ok) throw new HttpError(502, `Upstream returned ${resp.status}: ${text}`);
  try {
    return cors(new Response(text, { headers: JSON_HEADERS }));
  } catch {
    return json({ raw: text });
  }
}


async function authRegister(request: Request, env: Env, config: Config): Promise<Response> {
  const body = await jsonBody(request);
  const email = str(body.email).trim().toLowerCase();
  const password = str(body.password);
  const displayName = nullable(body.displayName || body.display_name);
  if (!EMAIL_RE.test(email)) throw new HttpError(400, "Invalid email");
  if (password.length < PASSWORD_MIN_LEN) throw new HttpError(400, `Password must be at least ${PASSWORD_MIN_LEN} characters`);

  const existing = await env.DB.prepare("SELECT * FROM useraccountrecord WHERE email = ?").bind(email).first<JsonObject>();
  if (existing) throw new HttpError(409, "Email already registered");

  const userId = crypto.randomUUID();
  const now = new Date().toISOString();
  const passwordHash = await hashPassword(password);
  await env.DB.prepare(
    "INSERT INTO useraccountrecord (user_id, email, password_hash, display_name, auth_provider, email_verified, created_at, updated_at) VALUES (?, ?, ?, ?, 'email', 0, ?, ?)"
  ).bind(userId, email, passwordHash, displayName, now, now).run();
  const user = await env.DB.prepare("SELECT * FROM useraccountrecord WHERE user_id = ?").bind(userId).first<JsonObject>();
  if (!user) throw new HttpError(500, "Failed to create account");
  return json(await emailAuthResponse(user, config));
}

async function authLogin(request: Request, env: Env, config: Config): Promise<Response> {
  const body = await jsonBody(request);
  const email = str(body.email).trim().toLowerCase();
  const password = str(body.password);
  const user = await env.DB.prepare("SELECT * FROM useraccountrecord WHERE email = ?").bind(email).first<JsonObject>();
  if (!user || !(await verifyPassword(password, str(user.password_hash)))) {
    throw new HttpError(401, "Invalid email or password");
  }
  return json(await emailAuthResponse(user, config));
}

async function emailAuthResponse(user: JsonObject, config: Config): Promise<JsonObject> {
  const userId = str(user.user_id);
  const expiresIn = config.int("SERVER_JWT_TTL_SEC", 86400);
  const accessToken = await signJwt(
    { iss: config.get("SERVER_JWT_ISSUER", "nola"), sub: `email:${userId}`, client_uuid: userId },
    config,
    expiresIn,
  );
  return {
    accessToken,
    expiresIn,
    userId,
    clientUuid: userId,
    email: user.email ?? null,
    displayName: user.display_name ?? null,
    authProvider: "email",
    user: mapUser(user),
  };
}

async function authMe(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  const user = await env.DB.prepare("SELECT user_id, email, display_name, auth_provider, apple_user_id, google_user_id, github_user_id, email_verified, created_at, updated_at FROM useraccountrecord WHERE user_id = ?")
    .bind(auth.clientUuid)
    .first<JsonObject>();
  if (!user) {
    return json({
      userId: hyphenUuid(auth.clientUuid),
      email: null,
      displayName: null,
      authProvider: "email",
    });
  }
  return json({
    userId: user.user_id,
    email: user.email ?? null,
    displayName: user.display_name ?? null,
    authProvider: user.auth_provider ?? "email",
  });
}

async function listModels(request: Request, env: Env, config: Config): Promise<Response> {
  const authHeader = request.headers.get("authorization");
  if (authHeader) {
    try {
      await requireAuth(request, config);
    } catch (error) {
      if (!request.headers.get("x-client-uuid")) throw error;
    }
  }
  const endpoint = config.get("TARGET_OPENAI_API_BASE").replace(/\/+$/, "");
  const apiKey = config.get("TARGET_OPENAI_API_KEY") || config.get("OPENAI_API_KEY");
  if (endpoint && apiKey) {
    const upstream = await fetch(`${endpoint}/models`, { headers: upstreamHeaders(config, apiKey) });
    if (upstream.ok) return cors(new Response(upstream.body, { status: upstream.status, headers: upstream.headers }));
  }
  const model = config.get("MODEL");
  return json({ object: "list", data: model ? [{ id: model, object: "model", owned_by: "nola" }] : [] });
}

async function chatCompletions(request: Request, env: Env, config: Config): Promise<Response> {
  await optionalAuth(request, config);
  const body = await jsonBody(request);
  if (config.bool("FORCE_DEFAULT_MODEL", false) && config.get("MODEL")) body.model = config.get("MODEL");
  if (!body.model && config.get("MODEL")) body.model = config.get("MODEL");
  const endpoint = defaultChatEndpoint(config);
  const apiKey = config.get("TARGET_OPENAI_API_KEY") || config.get("OPENAI_API_KEY");
  if (!endpoint || !apiKey) throw new HttpError(503, "Upstream LLM is not configured");
  const upstream = await fetch(endpoint, {
    method: "POST",
    headers: { ...upstreamHeaders(config, apiKey), "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  return cors(new Response(upstream.body, { status: upstream.status, headers: upstream.headers }));
}

async function listChatConversations(request: Request, env: Env, config: Config, url: URL): Promise<Response> {
  const auth = await requireAuth(request, config);
  const limit = Math.min(200, Math.max(1, Number(url.searchParams.get("limit") || 100)));
  const sinceRaw = url.searchParams.get("since") || null;
  let since: string | null = null;
  if (sinceRaw !== null) {
    // Require an ISO 8601-style date (YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS[.sss]Z) to
    // prevent silently wrong results from free-form strings that JS Date accepts.
    if (!/^\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2}(\.\d+)?Z?)?$/.test(sinceRaw)) {
      throw new HttpError(400, "Invalid 'since' parameter: expected ISO 8601 timestamp");
    }
    const parsed = new Date(sinceRaw);
    if (isNaN(parsed.getTime())) {
      throw new HttpError(400, "Invalid 'since' parameter: expected ISO 8601 timestamp");
    }
    since = parsed.toISOString();
  }
  const rows = since
    ? await env.DB.prepare("SELECT * FROM chatconversationrecord WHERE client_uuid = ? AND updated_at > ? ORDER BY updated_at DESC LIMIT ?")
        .bind(auth.clientUuid, since, limit)
        .all<JsonObject>()
    : await env.DB.prepare("SELECT * FROM chatconversationrecord WHERE client_uuid = ? ORDER BY updated_at DESC LIMIT ?")
        .bind(auth.clientUuid, limit)
        .all<JsonObject>();
  return json({ conversations: (rows.results || []).map((c) => ({ conversationId: c.conversation_id, title: c.title, createdAt: c.created_at, updatedAt: c.updated_at })) });
}

async function createChatConversation(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  const body = await jsonBody(request);
  const id = str(body.conversationId) || crypto.randomUUID();
  const title = str(body.title) || "新对话";
  const now = new Date().toISOString();
  await env.DB.prepare("INSERT OR IGNORE INTO chatconversationrecord (conversation_id, client_uuid, title, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
    .bind(id, auth.clientUuid, title, now, now)
    .run();
  return json({ conversationId: id, title, createdAt: now, updatedAt: now });
}

async function updateChatConversation(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const auth = await requireAuth(request, config);
  const body = await jsonBody(request);
  const id = decodeURIComponent(path.split("/").pop() || "");
  const now = new Date().toISOString();
  await env.DB.prepare("UPDATE chatconversationrecord SET title = ?, updated_at = ? WHERE client_uuid = ? AND conversation_id = ?")
    .bind(str(body.title), now, auth.clientUuid, id)
    .run();
  return json({ conversationId: id, title: str(body.title), updatedAt: now });
}

async function deleteChatConversation(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const auth = await requireAuth(request, config);
  const id = decodeURIComponent(path.split("/").pop() || "");
  await env.DB.batch([
    env.DB.prepare("DELETE FROM chatmessagerecord WHERE client_uuid = ? AND conversation_id = ?").bind(auth.clientUuid, id),
    env.DB.prepare("DELETE FROM chatconversationrecord WHERE client_uuid = ? AND conversation_id = ?").bind(auth.clientUuid, id),
  ]);
  return json({ accepted: true, conversationId: id });
}

async function listChatMessages(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const auth = await requireAuth(request, config);
  const id = decodeURIComponent(path.split("/v1/chat/conversations/")[1].split("/messages")[0]);
  const conv = await env.DB.prepare("SELECT conversation_id FROM chatconversationrecord WHERE client_uuid = ? AND conversation_id = ?")
    .bind(auth.clientUuid, id)
    .first<JsonObject>();
  if (!conv) throw new HttpError(404, "Conversation not found");
  const rows = await env.DB.prepare("SELECT id, role, content, created_at FROM chatmessagerecord WHERE client_uuid = ? AND conversation_id = ? ORDER BY id ASC")
    .bind(auth.clientUuid, id)
    .all<JsonObject>();
  return json({
    conversationId: id,
    messages: (rows.results || []).map((m) => ({
      id: m.id,
      role: m.role,
      content: m.content,
      createdAt: m.created_at,
    })),
  });
}

async function addChatMessage(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const auth = await requireAuth(request, config);
  const body = await jsonBody(request);
  const id = decodeURIComponent(path.split("/v1/chat/conversations/")[1].split("/messages")[0]);
  const now = new Date().toISOString();
  const role = str(body.role).trim().toLowerCase();
  if (!["user", "assistant", "system"].includes(role)) throw new HttpError(400, "Invalid role");
  const conv = await env.DB.prepare("SELECT conversation_id FROM chatconversationrecord WHERE client_uuid = ? AND conversation_id = ?")
    .bind(auth.clientUuid, id)
    .first<JsonObject>();
  if (!conv) throw new HttpError(404, "Conversation not found");
  const inserted = await env.DB.prepare("INSERT INTO chatmessagerecord (conversation_id, client_uuid, role, content, created_at) VALUES (?, ?, ?, ?, ?)")
    .bind(id, auth.clientUuid, role, str(body.content), now)
    .run();
  await env.DB.prepare("UPDATE chatconversationrecord SET updated_at = ? WHERE client_uuid = ? AND conversation_id = ?")
    .bind(now, auth.clientUuid, id)
    .run();
  return json({
    id: inserted.meta.last_row_id,
    conversationId: id,
    role,
    content: str(body.content),
    createdAt: now,
  });
}

interface CanonicalReplicaInput {
  operationId: string;
  entityType: string;
  entityId: string;
  revision: number;
  deleted: boolean;
  payload: unknown;
  originDeviceId: string;
}

function canonicalReplicaInput(value: unknown): CanonicalReplicaInput {
  if (!isObject(value)) throw new HttpError(400, "Each change must be an object");
  const operationId = str(value.operationId).trim();
  const entityType = str(value.entityType).trim();
  const entityId = str(value.entityId).trim();
  const revision = Number(value.revision);
  const deleted = value.deleted === true;
  const originDeviceId = str(value.originDeviceId).trim();
  if (!operationId || operationId.length > 200) throw new HttpError(400, "operationId is required and must not exceed 200 characters");
  if (!/^[a-z][a-z0-9._-]{0,79}$/.test(entityType)) throw new HttpError(400, "entityType is invalid");
  if (!entityId || entityId.length > 512) throw new HttpError(400, "entityId is required and must not exceed 512 characters");
  if (!Number.isSafeInteger(revision) || revision <= 0) throw new HttpError(400, "revision must be a positive integer");
  if (!originDeviceId || originDeviceId.length > 200) throw new HttpError(400, "originDeviceId is required and must not exceed 200 characters");
  if (!deleted && value.payload === undefined) throw new HttpError(400, "payload is required for a live replica");
  return { operationId, entityType, entityId, revision, deleted, payload: value.payload ?? null, originDeviceId };
}

function canonicalReplicaValue(row: JsonObject): JsonObject {
  let payload: unknown = null;
  try {
    payload = JSON.parse(str(row.payload_json || "null"));
  } catch {
    throw new HttpError(500, "Stored replica payload is invalid; cursor unchanged");
  }
  return {
    cursor: Number(row.change_cursor),
    operationId: str(row.operation_id),
    entityType: str(row.entity_type),
    entityId: str(row.entity_id),
    revision: Number(row.revision),
    deleted: Boolean(Number(row.deleted || 0)),
    payload,
    originDeviceId: str(row.origin_device_id),
    committedAt: str(row.committed_at),
  };
}

async function pushCanonicalReplicas(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  let body: JsonObject;
  try { body = await readReplicaBody(request); }
  catch (error) {
    if (error instanceof ReplicaTransportError) throw new HttpError(error.status, error.message);
    throw error;
  }
  const rawChanges = Array.isArray(body.changes) ? body.changes : [];
  if (!rawChanges.length) throw new HttpError(400, "changes must contain at least one item");
  if (rawChanges.length > 100) throw new HttpError(400, "changes must not contain more than 100 items");
  const changes = rawChanges.map(canonicalReplicaInput);
  const results: JsonObject[] = [];
  for (const change of changes) {
    const payloadJson = JSON.stringify(change.payload);
    const previousOperation = await env.DB.prepare(
      "SELECT * FROM canonicalreplicachangerecord WHERE client_uuid = ? AND operation_id = ?"
    ).bind(auth.clientUuid, change.operationId).first<JsonObject>();
    if (previousOperation) {
      const identical = str(previousOperation.entity_type) === change.entityType
        && str(previousOperation.entity_id) === change.entityId
        && Number(previousOperation.revision) === change.revision
        && Boolean(Number(previousOperation.deleted || 0)) === change.deleted
        && str(previousOperation.payload_json) === payloadJson;
      results.push({
        operationId: change.operationId,
        status: identical ? "idempotent" : "operation_conflict",
        currentRevision: Number(previousOperation.revision),
        cursor: Number(previousOperation.change_cursor),
      });
      continue;
    }
    const current = await env.DB.prepare(
      `SELECT * FROM canonicalreplicachangerecord
       WHERE client_uuid = ? AND entity_type = ? AND entity_id = ?
       ORDER BY revision DESC, change_cursor DESC LIMIT 1`
    ).bind(auth.clientUuid, change.entityType, change.entityId).first<JsonObject>();
    if (current && change.revision <= Number(current.revision)) {
      const unchanged = change.revision === Number(current.revision)
        && Boolean(Number(current.deleted || 0)) === change.deleted
        && str(current.payload_json) === payloadJson;
      results.push({
        operationId: change.operationId,
        status: unchanged ? "unchanged" : "revision_conflict",
        currentRevision: Number(current.revision),
        cursor: Number(current.change_cursor),
      });
      continue;
    }
    const committedAt = new Date().toISOString();
    const inserted = await env.DB.prepare(
      `INSERT INTO canonicalreplicachangerecord
       (client_uuid, operation_id, entity_type, entity_id, revision, deleted, payload_json, origin_device_id, committed_at)
       SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?
       WHERE ? > COALESCE((
         SELECT MAX(revision) FROM canonicalreplicachangerecord
         WHERE client_uuid = ? AND entity_type = ? AND entity_id = ?
       ), -1)`
    ).bind(
      auth.clientUuid, change.operationId, change.entityType, change.entityId,
      change.revision, change.deleted ? 1 : 0, payloadJson, change.originDeviceId, committedAt,
      change.revision, auth.clientUuid, change.entityType, change.entityId,
    ).run();
    if (!inserted.meta.changes) {
      const winner = await env.DB.prepare(
        `SELECT revision, change_cursor FROM canonicalreplicachangerecord
         WHERE client_uuid = ? AND entity_type = ? AND entity_id = ?
         ORDER BY revision DESC, change_cursor DESC LIMIT 1`
      ).bind(auth.clientUuid, change.entityType, change.entityId).first<JsonObject>();
      results.push({
        operationId: change.operationId,
        status: "revision_conflict",
        currentRevision: Number(winner?.revision ?? -1),
        cursor: Number(winner?.change_cursor ?? 0),
      });
      continue;
    }
    const stored = await env.DB.prepare(
      "SELECT change_cursor FROM canonicalreplicachangerecord WHERE client_uuid = ? AND operation_id = ?"
    ).bind(auth.clientUuid, change.operationId).first<JsonObject>();
    results.push({
      operationId: change.operationId,
      status: "accepted",
      currentRevision: change.revision,
      cursor: Number(stored?.change_cursor ?? 0),
    });
  }
  return json({ results, serverTime: new Date().toISOString() });
}

async function pullCanonicalReplicas(request: Request, env: Env, config: Config, url: URL): Promise<Response> {
  const auth = await requireAuth(request, config);
  const cursor = Number(url.searchParams.get("cursor") || 0);
  const requestedLimit = Number(url.searchParams.get("limit") || 200);
  if (!Number.isSafeInteger(requestedLimit) || requestedLimit <= 0) throw new HttpError(400, "limit must be a positive integer");
  const limit = Math.min(500, requestedLimit);
  const entityType = str(url.searchParams.get("entityType")).trim();
  if (!Number.isSafeInteger(cursor) || cursor < 0) throw new HttpError(400, "cursor must be a non-negative integer");
  if (entityType && !/^[a-z][a-z0-9._-]{0,79}$/.test(entityType)) throw new HttpError(400, "entityType is invalid");
  // Read only descriptors before selecting a bounded range of immutable bodies.
  const descriptors = entityType
    ? await env.DB.prepare(
        `SELECT change_cursor, length(CAST(payload_json AS BLOB)) AS payload_bytes FROM canonicalreplicachangerecord
         WHERE client_uuid = ? AND change_cursor > ? AND entity_type = ?
         ORDER BY change_cursor ASC LIMIT ?`
      ).bind(auth.clientUuid, cursor, entityType, limit).all<JsonObject>()
    : await env.DB.prepare(
        `SELECT change_cursor, length(CAST(payload_json AS BLOB)) AS payload_bytes FROM canonicalreplicachangerecord
         WHERE client_uuid = ? AND change_cursor > ?
         ORDER BY change_cursor ASC LIMIT ?`
      ).bind(auth.clientUuid, cursor, limit).all<JsonObject>();
  let end: number;
  try {
    end = replicaPageEnd((descriptors.results || []).map(row => ({
      cursor: Number(row.change_cursor), payloadBytes: Number(row.payload_bytes),
    })), cursor);
  } catch (error) {
    if (error instanceof ReplicaTransportError) throw new HttpError(error.status, error.message);
    throw error;
  }
  const rows = entityType
    ? await env.DB.prepare(
        `SELECT * FROM canonicalreplicachangerecord
         WHERE client_uuid = ? AND change_cursor > ? AND change_cursor <= ? AND entity_type = ?
         ORDER BY change_cursor ASC LIMIT ?`
      ).bind(auth.clientUuid, cursor, end, entityType, limit).all<JsonObject>()
    : await env.DB.prepare(
        `SELECT * FROM canonicalreplicachangerecord
         WHERE client_uuid = ? AND change_cursor > ? AND change_cursor <= ?
         ORDER BY change_cursor ASC LIMIT ?`
      ).bind(auth.clientUuid, cursor, end, limit).all<JsonObject>();
  const changes = (rows.results || []).map(canonicalReplicaValue);
  const nextCursor = changes.length ? Number(changes[changes.length - 1].cursor) : cursor;
  const more = entityType
    ? await env.DB.prepare(
        "SELECT 1 AS found FROM canonicalreplicachangerecord WHERE client_uuid = ? AND change_cursor > ? AND entity_type = ? LIMIT 1"
      ).bind(auth.clientUuid, nextCursor, entityType).first<JsonObject>()
    : await env.DB.prepare(
        "SELECT 1 AS found FROM canonicalreplicachangerecord WHERE client_uuid = ? AND change_cursor > ? LIMIT 1"
      ).bind(auth.clientUuid, nextCursor).first<JsonObject>();
  return json({ changes, nextCursor, hasMore: Boolean(more), serverTime: new Date().toISOString() });
}

async function registerDevice(request: Request, env: Env, config: Config): Promise<Response> {
  requirePush(config);
  const auth = await requireAuth(request, config);
  const body = await jsonBody(request);
  const token = str(body.deviceToken);
  if (!token) throw new HttpError(400, "deviceToken required");
  const deviceId = str(body.deviceId) || crypto.randomUUID();
  const now = new Date().toISOString();
  await env.DB.prepare(
    `INSERT INTO pushdevicerecord (device_id, client_uuid, device_token, platform, timezone, push_enabled, created_at, updated_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT(client_uuid, device_token) DO UPDATE SET timezone = excluded.timezone, push_enabled = excluded.push_enabled, updated_at = excluded.updated_at`
  ).bind(deviceId, auth.clientUuid, token, str(body.platform) || "ios", str(body.timezone) || "UTC", body.pushEnabled === false ? 0 : 1, now, now).run();
  return json({ deviceId });
}

async function syncSources(request: Request, env: Env, config: Config): Promise<Response> {
  requirePush(config);
  const auth = await requireAuth(request, config);
  const body = await jsonBody(request);
  const sources = Array.isArray(body.sources) ? body.sources as JsonObject[] : [];
  const deleted = Array.isArray(body.deletedSourceIds) ? body.deletedSourceIds.map(str).filter(Boolean) : [];
  const current = await env.DB.prepare("SELECT source_id FROM scheduledsourcerecord WHERE client_uuid = ?").bind(auth.clientUuid).all<JsonObject>();
  const finalIds = new Set((current.results || []).map((r) => str(r.source_id)).filter((id) => !deleted.includes(id)));
  for (const source of sources) finalIds.add(str(source.sourceId));
  if (finalIds.size > config.int("PUSH_MAX_SOURCES", 10)) throw new HttpError(400, `Too many scheduled sources (max=${config.int("PUSH_MAX_SOURCES", 10)}). Please remove some URLs.`);
  const now = new Date().toISOString();
  const statements: D1PreparedStatement[] = deleted.map((id) => env.DB.prepare("DELETE FROM scheduledsourcerecord WHERE client_uuid = ? AND source_id = ?").bind(auth.clientUuid, id));
  for (const source of sources) {
    statements.push(env.DB.prepare(
      `INSERT INTO scheduledsourcerecord (client_uuid, source_id, name, url, format, enabled, schedule_frequency, scheduled_time_iso, scheduled_weekday, timezone, user_prompt, updated_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(client_uuid, source_id) DO UPDATE SET name=excluded.name, url=excluded.url, format=excluded.format, enabled=excluded.enabled, schedule_frequency=excluded.schedule_frequency, scheduled_time_iso=excluded.scheduled_time_iso, scheduled_weekday=excluded.scheduled_weekday, timezone=excluded.timezone, user_prompt=excluded.user_prompt, updated_at=excluded.updated_at`
    ).bind(auth.clientUuid, str(source.sourceId), str(source.name) || "Untitled", str(source.url), str(source.format) || "html", source.enabled === false ? 0 : 1, str(source.scheduleFrequency) || "none", nullable(source.scheduledTimeISO), source.scheduledWeekday ?? null, str(source.timezone) || str(body.timezone) || "UTC", nullable(source.userPrompt), now));
  }
  if (statements.length) await env.DB.batch(statements);
  return json({ accepted: true, serverTime: now });
}

async function listSources(request: Request, env: Env, config: Config, url: URL): Promise<Response> {
  requirePush(config);
  const auth = await requireAuth(request, config);
  const limit = Math.min(200, Math.max(1, Number(url.searchParams.get("limit") || 50)));
  const rows = await env.DB.prepare("SELECT * FROM scheduledsourcerecord WHERE client_uuid = ? ORDER BY updated_at DESC LIMIT ?").bind(auth.clientUuid, limit).all<JsonObject>();
  return json({ sources: (rows.results || []).map(mapSource), serverTime: new Date().toISOString() });
}

async function deleteSource(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  requirePush(config);
  const auth = await requireAuth(request, config);
  const sourceId = decodeURIComponent(path.split("/")[3] || "");
  const result = await env.DB.prepare("DELETE FROM scheduledsourcerecord WHERE client_uuid = ? AND source_id = ?").bind(auth.clientUuid, sourceId).run();
  return json({ accepted: true, sourceId, status: result.meta.changes ? "deleted" : "already_deleted", deletedQueuedJobs: 0, serverTime: new Date().toISOString(), message: result.meta.changes ? "source deleted on server" : "source already absent on server" });
}

async function runSourceNow(request: Request, env: Env, config: Config, path: string, ctx: ExecutionContext): Promise<Response> {
  requirePush(config);
  const auth = await requireAuth(request, config);
  const sourceId = decodeURIComponent(path.split("/")[3] || "");
  const source = await env.DB.prepare("SELECT * FROM scheduledsourcerecord WHERE client_uuid = ? AND source_id = ?").bind(auth.clientUuid, sourceId).first<JsonObject>();
  if (!source) throw new HttpError(404, "source not found");
  if (!Number(source.enabled)) throw new HttpError(400, "source is disabled");
  const jobId = crypto.randomUUID();
  const resultId = crypto.randomUUID();
  const now = new Date().toISOString();
  await env.DB.prepare("INSERT INTO scheduledjobrecord (job_id, client_uuid, source_id, scheduled_at, run_at, status, result_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'running', ?, ?, ?)")
    .bind(jobId, auth.clientUuid, sourceId, now, now, resultId, now, now)
    .run();
  ctx.waitUntil(executeRunNow(env, config, source, jobId, resultId));
  return json({ accepted: true, jobId, resultId, status: "running", note: "server_processing", waitedSec: 0, stage: "queued" });
}

interface ScheduledSlot {
  key: string;
  disableAfterClaim: boolean;
}

export function scheduledSlotForSource(source: JsonObject, now: Date): ScheduledSlot | null {
  const frequency = str(source.schedule_frequency).trim().toLowerCase();
  const rawTime = str(source.scheduled_time_iso).trim();
  const timezone = str(source.timezone).trim() || "UTC";
  if (!rawTime || !["once", "daily", "weekly"].includes(frequency)) return null;

  if (frequency === "once") {
    const target = new Date(rawTime);
    if (!Number.isFinite(target.getTime())) return null;
    const targetMinute = Math.floor(target.getTime() / 60_000);
    const nowMinute = Math.floor(now.getTime() / 60_000);
    return targetMinute === nowMinute
      ? { key: target.toISOString().slice(0, 16), disableAfterClaim: true }
      : null;
  }

  const match = /^(\d{1,2}):(\d{2})$/.exec(rawTime);
  if (!match) return null;
  const requestedHour = Number(match[1]);
  const requestedMinute = Number(match[2]);
  if (requestedHour > 23 || requestedMinute > 59) return null;

  let parts: Intl.DateTimeFormatPart[];
  try {
    parts = new Intl.DateTimeFormat("en-US", {
      timeZone: timezone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
      weekday: "short",
    }).formatToParts(now);
  } catch {
    return null;
  }
  const value = (type: Intl.DateTimeFormatPartTypes): string => parts.find((part) => part.type === type)?.value || "";
  if (Number(value("hour")) !== requestedHour || Number(value("minute")) !== requestedMinute) return null;

  if (frequency === "weekly") {
    const weekday = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].indexOf(value("weekday")) + 1;
    const requestedWeekday = Number(source.scheduled_weekday);
    if (weekday < 1 || requestedWeekday !== weekday) return null;
  }

  return {
    key: `${value("year")}-${value("month")}-${value("day")}T${String(requestedHour).padStart(2, "0")}:${String(requestedMinute).padStart(2, "0")}@${timezone}`,
    disableAfterClaim: false,
  };
}

async function runDueScheduledSources(env: Env, config: Config, now: Date): Promise<void> {
  const rows = await env.DB.prepare(
    `SELECT * FROM scheduledsourcerecord
     WHERE enabled = 1
       AND schedule_frequency IN ('once', 'daily', 'weekly')
       AND scheduled_time_iso IS NOT NULL
     ORDER BY updated_at ASC
     LIMIT ?`
  ).bind(Math.max(1, config.int("PUSH_SCHEDULER_BATCH_SIZE", 100))).all<JsonObject>();

  const executions: Promise<void>[] = [];
  for (const source of rows.results || []) {
    const slot = scheduledSlotForSource(source, now);
    if (!slot) continue;
    const identity = `${str(source.client_uuid)}:${str(source.source_id)}:${slot.key}`;
    const jobId = `cron:${identity}`;
    const resultId = `cron-result:${identity}`;
    const scheduledAt = now.toISOString();
    const claim = await env.DB.prepare(
      `INSERT OR IGNORE INTO scheduledjobrecord
       (job_id, client_uuid, source_id, scheduled_at, run_at, status, result_id, created_at, updated_at)
       VALUES (?, ?, ?, ?, ?, 'running', ?, ?, ?)`
    ).bind(jobId, source.client_uuid, source.source_id, scheduledAt, scheduledAt, resultId, scheduledAt, scheduledAt).run();
    if (!claim.meta.changes) continue;
    if (slot.disableAfterClaim) {
      await env.DB.prepare("UPDATE scheduledsourcerecord SET enabled = 0, updated_at = ? WHERE client_uuid = ? AND source_id = ?")
        .bind(scheduledAt, source.client_uuid, source.source_id)
        .run();
    }
    executions.push(executeRunNow(env, config, source, jobId, resultId));
  }
  await Promise.allSettled(executions);
}

async function executeRunNow(env: Env, config: Config, source: JsonObject, jobId: string, resultId: string): Promise<void> {
  const now = new Date();
  try {
    const target = str(source.url);
    const fetched = await fetch(target, { redirect: "follow" });
    const text = (await fetched.text()).slice(0, config.int("PUSH_MAX_FETCH_BYTES", 1024 * 1024));
    const summary = await summarize(config, source, text);
    await env.DB.batch([
      env.DB.prepare("INSERT INTO scheduledresultrecord (result_id, job_id, client_uuid, source_id, scheduled_at, fetched_at, status, llm_summary_text, events_json, diagnostics_json, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, 'success', ?, '[]', ?, ?, ?)")
        .bind(resultId, jobId, source.client_uuid, source.source_id, now.toISOString(), new Date().toISOString(), summary, JSON.stringify({ httpStatus: fetched.status }), new Date().toISOString(), new Date(Date.now() + 3 * 86400_000).toISOString()),
      env.DB.prepare("UPDATE scheduledjobrecord SET status = 'success', result_id = ?, updated_at = ? WHERE job_id = ?").bind(resultId, new Date().toISOString(), jobId),
    ]);
    await dispatchResultPush(env, config, str(source.client_uuid), str(source.source_id), jobId, resultId, now.toISOString(), summary, "success");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const completedAt = new Date().toISOString();
    await env.DB.batch([
      env.DB.prepare("INSERT OR IGNORE INTO scheduledresultrecord (result_id, job_id, client_uuid, source_id, scheduled_at, fetched_at, status, llm_summary_text, llm_explain, events_json, diagnostics_json, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, 'failed', ?, ?, '[]', ?, ?, ?)")
        .bind(resultId, jobId, source.client_uuid, source.source_id, now.toISOString(), completedAt, message || "run failed", message, JSON.stringify({ reasonCode: "worker_run_failed", reasonMessage: message }), completedAt, new Date(Date.now() + 3 * 86400_000).toISOString()),
      env.DB.prepare("UPDATE scheduledjobrecord SET status = 'failed', result_id = ?, error_message = ?, updated_at = ? WHERE job_id = ?")
        .bind(resultId, message, completedAt, jobId),
    ]);
    await dispatchResultPush(env, config, str(source.client_uuid), str(source.source_id), jobId, resultId, now.toISOString(), message || "run failed", "failed");
  }
}

async function dispatchResultPush(env: Env, config: Config, clientUuid: string, sourceId: string, jobId: string, resultId: string, scheduledAt: string, summary: string, status: string): Promise<void> {
  if (!config.bool("PUSH_FEATURE_ENABLED", false)) return;
  const notificationTitle = "定时动作已完成";
  const notificationBody = str(summary).slice(0, 180) || "定时动作结果已生成，点击进入查看";
  const dedupeKey = `push_summary:${resultId}`;
  await upsertNotification(env.DB, {
    clientUuid,
    title: notificationTitle,
    body: notificationBody,
    source: "scheduled_action",
    type: "scheduled_result",
    dedupeKey,
    payload: {
      type: "scheduled_url_result",
      jobId,
      sourceId,
      resultId,
      scheduledAt,
      status,
      job_id: jobId,
      source_id: sourceId,
      result_id: resultId,
      scheduled_at: scheduledAt,
      status_code: status,
    },
    createdAt: scheduledAt,
  });
  const devices = await env.DB.prepare("SELECT device_token FROM pushdevicerecord WHERE client_uuid = ? AND push_enabled = 1")
    .bind(clientUuid)
    .all<JsonObject>();
  const deviceRows = devices.results || [];
  if (!deviceRows.length) {
    await env.DB.prepare("UPDATE scheduledresultrecord SET silent_push_sent = 1, silent_push_sent_at = ?, visible_fallback_sent = 1 WHERE client_uuid = ? AND result_id = ?")
      .bind(new Date().toISOString(), clientUuid, resultId)
      .run();
    return;
  }

  const deliveryMode = config.get("PUSH_DELIVERY_MODE", "alert") === "silent" ? "silent" : "alert";
  const pushType = deliveryMode === "silent" ? "background" : "alert";
  const priority = deliveryMode === "silent" ? "5" : "10";
  const payload = deliveryMode === "silent"
    ? {
      aps: { "content-available": 1 },
      type: "scheduled_url_result",
      jobId,
      sourceId,
      resultId,
      scheduledAt,
      job_id: jobId,
      source_id: sourceId,
      result_id: resultId,
      scheduled_at: scheduledAt,
    }
    : {
      aps: { alert: { title: notificationTitle, body: notificationBody }, sound: "default" },
      type: "scheduled_url_alert_summary",
      jobId,
      sourceId,
      resultId,
      scheduledAt,
      status,
      job_id: jobId,
      source_id: sourceId,
      result_id: resultId,
      scheduled_at: scheduledAt,
      status_code: status,
    };

  let sent = false;
  for (const device of deviceRows) {
    const token = str(device.device_token);
    if (!token) continue;
    try {
      await sendApns(config, token, payload, pushType, priority);
      sent = true;
    } catch (error) {
      console.warn("APNs delivery failed", error instanceof Error ? error.message : String(error));
    }
  }
  if (sent) {
    await env.DB.prepare("UPDATE scheduledresultrecord SET silent_push_sent = 1, silent_push_sent_at = ? WHERE client_uuid = ? AND result_id = ?")
      .bind(new Date().toISOString(), clientUuid, resultId)
      .run();
  }
}

async function summarize(config: Config, source: JsonObject, text: string): Promise<string> {
  const endpoint = defaultChatEndpoint(config);
  const apiKey = config.get("TARGET_OPENAI_API_KEY") || config.get("OPENAI_API_KEY");
  if (!endpoint || !apiKey) return text.slice(0, 1200);
  const resp = await fetch(endpoint, {
    method: "POST",
    headers: { ...upstreamHeaders(config, apiKey), "content-type": "application/json" },
    body: JSON.stringify({
      model: config.get("MODEL"),
      messages: [
        { role: "system", content: "Summarize the fetched source for a scheduled Nola notification. Be concise." },
        { role: "user", content: `Source: ${str(source.name)}\nPrompt: ${str(source.user_prompt)}\n\n${text.slice(0, 12000)}` },
      ],
      stream: false,
      max_tokens: config.int("PUSH_LLM_MAX_OUTPUT_TOKENS", 800),
    }),
  });
  if (!resp.ok) return text.slice(0, 1200);
  const data = await resp.json<JsonObject>();
  const choices = Array.isArray(data.choices) ? data.choices as JsonObject[] : [];
  const message = choices[0]?.message as JsonObject | undefined;
  return str(message?.content) || text.slice(0, 1200);
}

async function listResults(request: Request, env: Env, config: Config, url: URL): Promise<Response> {
  const auth = await requireAuth(request, config);
  const limit = Math.min(200, Math.max(1, Number(url.searchParams.get("limit") || 50)));
  const rows = await env.DB.prepare(
    `SELECT r.*, s.name AS source_name, s.url AS source_url, s.format AS source_format
     FROM scheduledresultrecord r
     LEFT JOIN scheduledsourcerecord s ON s.client_uuid = r.client_uuid AND s.source_id = r.source_id
     WHERE r.client_uuid = ?
     ORDER BY r.created_at DESC
     LIMIT ?`
  ).bind(auth.clientUuid, limit).all<JsonObject>();
  return json({
    serverTime: new Date().toISOString(),
    count: (rows.results || []).length,
    results: (rows.results || []).map(mapResultSummary),
  });
}

async function getResult(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const auth = await requireAuth(request, config);
  const resultId = decodeURIComponent(path.split("/")[3] || "");
  const row = await env.DB.prepare(
    `SELECT r.*, s.name AS source_name, s.url AS source_url, s.format AS source_format
     FROM scheduledresultrecord r
     LEFT JOIN scheduledsourcerecord s ON s.client_uuid = r.client_uuid AND s.source_id = r.source_id
     WHERE r.client_uuid = ? AND r.result_id = ?`
  ).bind(auth.clientUuid, resultId).first<JsonObject>();
  if (!row) throw new HttpError(404, "result not found");
  return json(mapResult(row));
}

async function ackResult(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const auth = await requireAuth(request, config);
  const resultId = decodeURIComponent(path.split("/")[3] || "");
  const body: JsonObject = await jsonBody(request).catch(() => ({}));
  const deviceId = str(body.deviceId || body.device_id).trim();
  if (!deviceId) throw new HttpError(400, "deviceId required");
  const now = new Date().toISOString();
  await env.DB.batch([
    env.DB.prepare("INSERT OR IGNORE INTO scheduledresultackrecord (result_id, client_uuid, device_id, received_at, status) VALUES (?, ?, ?, ?, ?)")
      .bind(resultId, auth.clientUuid, deviceId, now, str(body.status) || "pulled_and_persisted"),
    env.DB.prepare("UPDATE scheduledresultrecord SET ack_count = ack_count + 1 WHERE client_uuid = ? AND result_id = ?").bind(auth.clientUuid, resultId),
  ]);
  return json({ ok: true, accepted: true, resultId, serverTime: now });
}

interface NotificationUpsert {
  clientUuid: string;
  title: string;
  body?: string;
  source: string;
  type: string;
  dedupeKey: string;
  payload?: JsonObject;
  createdAt?: string;
}

async function upsertNotification(db: D1Database, item: NotificationUpsert): Promise<void> {
  const now = new Date().toISOString();
  await db.prepare(
    `INSERT INTO notificationrecord (notification_id, client_uuid, title, body, source, notification_type, dedupe_key, payload_json, is_read, created_at, updated_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)
     ON CONFLICT(client_uuid, dedupe_key) DO UPDATE SET
       title = excluded.title,
       body = excluded.body,
       source = excluded.source,
       notification_type = excluded.notification_type,
       payload_json = excluded.payload_json,
       updated_at = excluded.updated_at`
  ).bind(
    crypto.randomUUID(),
    item.clientUuid,
    item.title,
    item.body ?? null,
    item.source,
    item.type,
    item.dedupeKey,
    JSON.stringify(item.payload || {}),
    item.createdAt || now,
    now,
  ).run();
}

async function listNotifications(request: Request, env: Env, config: Config, url: URL): Promise<Response> {
  requirePush(config);
  const auth = await requireAuth(request, config);
  const limit = Math.min(200, Math.max(1, Number(url.searchParams.get("limit") || 50)));
  const offset = Math.max(0, Number(url.searchParams.get("offset") || 0));
  const unreadOnly = ["1", "true", "yes"].includes((url.searchParams.get("unread_only") || "").toLowerCase());
  const rows = unreadOnly
    ? await env.DB.prepare(
      "SELECT * FROM notificationrecord WHERE client_uuid = ? AND is_read = 0 ORDER BY created_at DESC, notification_id DESC LIMIT ? OFFSET ?"
    ).bind(auth.clientUuid, limit, offset).all<JsonObject>()
    : await env.DB.prepare(
      "SELECT * FROM notificationrecord WHERE client_uuid = ? ORDER BY created_at DESC, notification_id DESC LIMIT ? OFFSET ?"
    ).bind(auth.clientUuid, limit, offset).all<JsonObject>();
  const unread = await unreadNotificationCount(env.DB, auth.clientUuid);
  return json({
    notifications: (rows.results || []).map(mapNotification),
    count: (rows.results || []).length,
    unreadCount: unread,
  });
}

async function notificationUnreadCount(request: Request, env: Env, config: Config): Promise<Response> {
  requirePush(config);
  const auth = await requireAuth(request, config);
  return json({ unreadCount: await unreadNotificationCount(env.DB, auth.clientUuid) });
}

async function markNotificationRead(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  requirePush(config);
  const auth = await requireAuth(request, config);
  const id = decodeURIComponent(path.split("/")[3] || "");
  const now = new Date().toISOString();
  const result = await env.DB.prepare(
    "UPDATE notificationrecord SET is_read = 1, read_at = COALESCE(read_at, ?), updated_at = ? WHERE client_uuid = ? AND (notification_id = ? OR dedupe_key = ?)"
  ).bind(now, now, auth.clientUuid, id, id).run();
  if (!result.meta.changes) throw new HttpError(404, "notification not found");
  return json({ ok: true, unreadCount: await unreadNotificationCount(env.DB, auth.clientUuid) });
}

async function markAllNotificationsRead(request: Request, env: Env, config: Config): Promise<Response> {
  requirePush(config);
  const auth = await requireAuth(request, config);
  const now = new Date().toISOString();
  await env.DB.prepare(
    "UPDATE notificationrecord SET is_read = 1, read_at = COALESCE(read_at, ?), updated_at = ? WHERE client_uuid = ? AND is_read = 0"
  ).bind(now, now, auth.clientUuid).run();
  return json({ ok: true, unreadCount: 0 });
}

async function unreadNotificationCount(db: D1Database, clientUuid: string): Promise<number> {
  const row = await db.prepare(
    "SELECT COUNT(*) AS count FROM notificationrecord WHERE client_uuid = ? AND is_read = 0"
  ).bind(clientUuid).first<JsonObject>();
  return Number(row?.count || 0);
}

function mapNotification(row: JsonObject): JsonObject {
  let payload: JsonObject = {};
  try {
    const parsed = JSON.parse(str(row.payload_json || "{}"));
    if (isObject(parsed)) payload = parsed;
  } catch {
    payload = {};
  }
  return {
    id: str(row.notification_id),
    title: str(row.title),
    body: row.body ?? null,
    source: str(row.source || "system"),
    type: str(row.notification_type || "message"),
    dedupeKey: str(row.dedupe_key),
    payload,
    isRead: Boolean(Number(row.is_read || 0)),
    createdAt: str(row.created_at),
    readAt: row.read_at ?? null,
    updatedAt: str(row.updated_at || row.created_at),
  };
}

async function mobileLogin(request: Request, env: Env, config: Config): Promise<Response> {
  const body = await jsonBody(request);
  const email = str(body.email).trim().toLowerCase();
  const password = str(body.password);
  const user = await env.DB.prepare("SELECT * FROM useraccountrecord WHERE email = ?").bind(email).first<JsonObject>();
  if (!user || !(await verifyPassword(password, str(user.password_hash)))) {
    throw new HttpError(401, "Invalid email or password");
  }
  const auth = await emailAuthResponse(user, config);
  return json({
    token: auth.accessToken,
    token_type: "Bearer",
    expires_in: auth.expiresIn,
    local_user_id: auth.userId,
    display_name: auth.displayName,
    email: auth.email,
  });
}

async function mobileMe(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  const user = await env.DB.prepare("SELECT user_id, display_name, email FROM useraccountrecord WHERE user_id = ?").bind(auth.clientUuid).first<JsonObject>();
  if (user) {
    return json({
      local_user_id: user.user_id,
      display_name: user.display_name || "",
      email: user.email,
    });
  }
  return json({
    local_user_id: hyphenUuid(auth.clientUuid),
    display_name: "",
    email: null,
  });
}

async function mobileAgentSettings(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  const token = bearerToken(request);
  const settings = agentSettingsToDict(await getOrCreateAgentSettings(env.DB, auth.clientUuid));
  const payload = {
    uploaded_skills: settings.uploaded_skills || [],
    uploaded_documents: settings.uploaded_documents || [],
    mcp_servers: settings.mcp_servers || [],
    enable_local_dev_tools: false,
  };
  return json(await encryptMobilePayload(payload, token));
}

async function mobileAttachmentParse(request: Request, config: Config): Promise<Response> {
  const token = bearerToken(request);
  if (request.headers.get("X-Nola-Mobile-Crypto") !== "aes-gcm-v1") throw new HttpError(400, "encrypted payload required");
  const envelope = await jsonBody(request);
  const data = await decryptMobilePayload(envelope, token);
  const filename = str(data.filename || "attachment");
  const mimeType = str(data.mime_type || data.mimeType || "application/octet-stream");
  const encoded = str(data.data_base64);
  if (!encoded) throw new HttpError(400, "data_base64 required");
  const raw = bytesFromBase64Url(encoded.replace(/\+/g, "-").replace(/\//g, "_"));
  if (raw.byteLength > config.int("MOBILE_ATTACHMENT_PARSE_MAX_BYTES", 20 * 1024 * 1024)) {
    throw new HttpError(413, `attachment too large; max ${config.int("MOBILE_ATTACHMENT_PARSE_MAX_BYTES", 20 * 1024 * 1024)} bytes`);
  }
  const parsed = parseAttachmentText(raw, filename, mimeType);
  if (!parsed) throw new HttpError(422, "no trustworthy text extracted; use a platform document parser or OCR for binary files");
  return json(await encryptMobilePayload({
    filename,
    parser: parsed.parser,
    text: parsed.text.slice(0, 40000),
    truncated: parsed.text.length > 40000,
  }, token));
}

async function agentSettingsGet(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  const row = await getOrCreateAgentSettings(env.DB, auth.clientUuid);
  return json(agentSettingsToDict(row));
}

async function agentSettingsPost(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  const data = await jsonBody(request);
  const row = await getOrCreateAgentSettings(env.DB, auth.clientUuid);
  const updated = {
    enable_local_dev_tools: data.enable_local_dev_tools === undefined ? row.enable_local_dev_tools : (Booleanish(data.enable_local_dev_tools) ? 1 : 0),
    allowed_dirs_json: data.allowed_dirs === undefined ? row.allowed_dirs_json : JSON.stringify(Array.isArray(data.allowed_dirs) ? data.allowed_dirs : []),
    bash_prefixes_json: (data.bash_prefixes === undefined && data.bash_allowed_prefixes === undefined)
      ? row.bash_prefixes_json
      : JSON.stringify(Array.isArray(data.bash_prefixes) ? data.bash_prefixes : (Array.isArray(data.bash_allowed_prefixes) ? data.bash_allowed_prefixes : [])),
    agent_llm_endpoint: data.agent_llm_endpoint === undefined ? row.agent_llm_endpoint : nullable(data.agent_llm_endpoint),
    agent_llm_api_key: data.agent_llm_api_key === undefined ? row.agent_llm_api_key : nullable(data.agent_llm_api_key),
    agent_llm_model: data.agent_llm_model === undefined ? row.agent_llm_model : nullable(data.agent_llm_model),
    response_style: data.response_style === undefined ? (row.response_style || "casual") : (nullable(data.response_style) || "casual"),
    long_term_memory: data.long_term_memory === undefined ? row.long_term_memory : (typeof data.long_term_memory === "string" ? data.long_term_memory : JSON.stringify(data.long_term_memory ?? "")),
    user_preferences: data.user_preferences === undefined ? row.user_preferences : (typeof data.user_preferences === "string" ? data.user_preferences : JSON.stringify(data.user_preferences ?? "")),
    mcp_servers_json: data.mcp_servers === undefined ? row.mcp_servers_json : JSON.stringify(Array.isArray(data.mcp_servers) ? data.mcp_servers : []),
    uploaded_skills_json: data.uploaded_skills === undefined ? row.uploaded_skills_json : JSON.stringify(Array.isArray(data.uploaded_skills) ? data.uploaded_skills : []),
    uploaded_documents_json: data.uploaded_documents === undefined ? row.uploaded_documents_json : JSON.stringify(Array.isArray(data.uploaded_documents) ? data.uploaded_documents : []),
    updated_at: new Date().toISOString(),
  };
  await env.DB.prepare(
    `UPDATE agentsettingrecord SET
      enable_local_dev_tools = ?,
      allowed_dirs_json = ?,
      bash_prefixes_json = ?,
      agent_llm_endpoint = ?,
      agent_llm_api_key = ?,
      agent_llm_model = ?,
      response_style = ?,
      long_term_memory = ?,
      user_preferences = ?,
      mcp_servers_json = ?,
      uploaded_skills_json = ?,
      uploaded_documents_json = ?,
      updated_at = ?
     WHERE id = ?`
  ).bind(
    updated.enable_local_dev_tools,
    updated.allowed_dirs_json,
    updated.bash_prefixes_json,
    updated.agent_llm_endpoint,
    updated.agent_llm_api_key,
    updated.agent_llm_model,
    updated.response_style,
    updated.long_term_memory,
    updated.user_preferences,
    updated.mcp_servers_json,
    updated.uploaded_skills_json,
    updated.uploaded_documents_json,
    updated.updated_at,
    row.id,
  ).run();
  const latest = await getOrCreateAgentSettings(env.DB, auth.clientUuid);
  return json(agentSettingsToDict(latest));
}

async function permissionRulesGet(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  const rows = await env.DB.prepare("SELECT id, tool_name, behavior, match FROM agentpermissionrulerecord WHERE client_uuid = ? ORDER BY id").bind(auth.clientUuid).all<JsonObject>();
  return json({ rules: rows.results || [] });
}

async function permissionRulesPost(request: Request, env: Env, config: Config): Promise<Response> {
  const auth = await requireAuth(request, config);
  const body = await jsonBody(request);
  const toolName = str(body.tool_name).trim();
  const behavior = str(body.behavior || "ask").trim().toLowerCase();
  const match = str(body.match || "*").trim() || "*";
  if (!toolName) throw new HttpError(400, "tool_name required");
  if (!["allow", "deny", "ask"].includes(behavior)) throw new HttpError(400, "behavior must be 'allow', 'deny', or 'ask'");
  const now = new Date().toISOString();
  const result = await env.DB.prepare("INSERT INTO agentpermissionrulerecord (client_uuid, tool_name, behavior, match, created_at) VALUES (?, ?, ?, ?, ?)")
    .bind(auth.clientUuid, toolName, behavior, match, now)
    .run();
  return json({ id: result.meta.last_row_id, tool_name: toolName, behavior, match });
}

async function permissionRulesDelete(request: Request, env: Env, config: Config, path: string): Promise<Response> {
  const auth = await requireAuth(request, config);
  const ruleId = Number(path.split("/").pop() || 0);
  if (!Number.isFinite(ruleId) || ruleId <= 0) throw new HttpError(400, "Invalid rule id");
  const result = await env.DB.prepare("DELETE FROM agentpermissionrulerecord WHERE id = ? AND client_uuid = ?").bind(ruleId, auth.clientUuid).run();
  if (!result.meta.changes) throw new HttpError(404, "Rule not found");
  return json({ deleted: true });
}

function agentToolsList(): Response {
  return json({
    tools: [
      {
        name: "official_chat",
        description: "Official backend LLM streaming. Local filesystem, shell, and browser tools are not available on the official backend.",
        category: "official",
        is_read_only: true,
        is_concurrency_safe: true,
        parameters: { type: "object", properties: {} },
      },
    ],
  });
}

async function debugPushSilent(request: Request, config: Config): Promise<Response> {
  requirePush(config);
  requireDebugAdmin(request, config);
  const body = await jsonBody(request);
  const deviceToken = str(body.deviceToken).trim();
  if (!deviceToken) throw new HttpError(400, "deviceToken required");
  const payload = {
    aps: { "content-available": 1 },
    type: str(body.type || "debug_silent"),
    ...(isObject(body.customData) ? body.customData as JsonObject : {}),
  };
  await sendApns(config, deviceToken, payload, "background", str(body.priority || "5"));
  return json({ ok: true });
}

async function debugPushAlert(request: Request, config: Config): Promise<Response> {
  requirePush(config);
  requireDebugAdmin(request, config);
  const body = await jsonBody(request);
  const deviceToken = str(body.deviceToken).trim();
  if (!deviceToken) throw new HttpError(400, "deviceToken required");
  const payload = {
    aps: { alert: { title: str(body.title || "Nola"), body: str(body.body || "debug alert") }, sound: "default" },
    type: str(body.type || "debug_alert"),
    ...(isObject(body.customData) ? body.customData as JsonObject : {}),
  };
  await sendApns(config, deviceToken, payload, "alert", str(body.priority || "10"));
  return json({ ok: true });
}

async function debugApnsValidateDevice(request: Request, config: Config): Promise<Response> {
  requirePush(config);
  requireDebugAdmin(request, config);
  const body = await jsonBody(request);
  const deviceToken = str(body.deviceToken).trim();
  if (!deviceToken) throw new HttpError(400, "deviceToken required");
  const pushType = str(body.pushType || "background").toLowerCase();
  const priority = str(body.priority || (pushType === "background" ? "5" : "10"));
  const topic = str(body.topic || config.get("APNS_TOPIC"));
  const payload = pushType === "background"
    ? { aps: { "content-available": 1 }, type: "debug_validate_device" }
    : { aps: { alert: { title: "Nola Debug", body: "validate_device" }, sound: "default" }, type: "debug_validate_device" };
  try {
    const result = await sendApns(config, deviceToken, payload, pushType as "background" | "alert", priority, topic);
    return json({
      serverTimeUtc: new Date().toISOString(),
      request: { pushType, priority, topic, useSandbox: config.bool("APNS_USE_SANDBOX", false), deviceTokenTail: deviceToken.slice(-8) },
      result,
    });
  } catch (error) {
    return json({
      serverTimeUtc: new Date().toISOString(),
      request: { pushType, priority, topic, useSandbox: config.bool("APNS_USE_SANDBOX", false), deviceTokenTail: deviceToken.slice(-8) },
      result: { ok: false, accepted: false, status: 500, reason: error instanceof Error ? error.message : String(error) },
    });
  }
}

async function debugApnsStatus(request: Request, config: Config): Promise<Response> {
  requirePush(config);
  requireDebugAdmin(request, config);
  const key = normalizePem(config.get("APNS_AUTH_KEY_P8")) || "";
  const lines = key ? key.split("\n").map((line) => line.trim()).filter(Boolean) : [];
  let jwtInfo: JsonObject;
  try {
    const token = await apnsJwt(config);
    jwtInfo = {
      ok: true,
      tokenLength: token.length,
      header: parseJwtHeader(token),
      claims: parseJwtClaims(token),
    };
  } catch (error) {
    jwtInfo = { ok: false, error: error instanceof Error ? error.message : String(error) };
  }
  return json({
    serverTimeUtc: new Date().toISOString(),
    push: {
      featureEnabled: config.bool("PUSH_FEATURE_ENABLED", false),
      schedulerEnabled: config.bool("PUSH_SCHEDULER_ENABLED", false),
      deliveryMode: config.get("PUSH_DELIVERY_MODE", "alert"),
      executeLeadMin: config.int("PUSH_EXECUTE_LEAD_MIN", 5),
      visibleFallbackGraceMin: config.int("PUSH_VISIBLE_FALLBACK_GRACE_MIN", 10),
    },
    apns: {
      teamIdPresent: Boolean(config.get("APNS_TEAM_ID")),
      keyIdPresent: Boolean(config.get("APNS_KEY_ID")),
      topic: config.get("APNS_TOPIC"),
      useSandbox: config.bool("APNS_USE_SANDBOX", false),
      key: {
        present: Boolean(key),
        numLines: lines.length,
        firstLine: lines[0] || "",
        lastLine: lines[lines.length - 1] || "",
        hasBegin: lines.some((line) => line.includes("BEGIN PRIVATE KEY")),
        hasEnd: lines.some((line) => line.includes("END PRIVATE KEY")),
      },
      jwt: jwtInfo,
    },
  });
}

async function debugPushClient(request: Request, env: Env, config: Config, clientUuidRaw: string): Promise<Response> {
  requirePush(config);
  requireDebugAdmin(request, config);
  const clientUuid = normalizeUuid(clientUuidRaw);
  const [sources, devices, jobs, results] = await Promise.all([
    env.DB.prepare("SELECT * FROM scheduledsourcerecord WHERE client_uuid = ? ORDER BY updated_at DESC LIMIT 50").bind(clientUuid).all<JsonObject>(),
    env.DB.prepare("SELECT * FROM pushdevicerecord WHERE client_uuid = ? ORDER BY updated_at DESC LIMIT 50").bind(clientUuid).all<JsonObject>(),
    env.DB.prepare("SELECT job_id, source_id, scheduled_at, status, attempt FROM scheduledjobrecord WHERE client_uuid = ? AND status = 'queued' ORDER BY scheduled_at ASC LIMIT 20").bind(clientUuid).all<JsonObject>(),
    env.DB.prepare("SELECT result_id, source_id, status, ack_count, created_at, silent_push_sent, visible_fallback_sent FROM scheduledresultrecord WHERE client_uuid = ? ORDER BY created_at DESC LIMIT 20").bind(clientUuid).all<JsonObject>(),
  ]);
  return json({
    clientUuid: clientUuid,
    serverTimeUtc: new Date().toISOString(),
    sources: (sources.results || []).map((row) => ({
      ...mapSource(row),
      userPromptLen: str(row.user_prompt || "").length,
      nextScheduledAt: null,
    })),
    devices: (devices.results || []).map((row) => ({
      deviceId: row.device_id,
      platform: row.platform,
      timezone: row.timezone,
      pushEnabled: Boolean(Number(row.push_enabled ?? 0)),
      tokenSuffix: str(row.device_token).slice(-12),
      updatedAt: row.updated_at,
    })),
    queuedJobs: jobs.results || [],
    recentResults: results.results || [],
  });
}

function requirePush(config: Config): void {
  if (!config.bool("PUSH_FEATURE_ENABLED", false)) throw new HttpError(503, "Push feature disabled");
}

async function requireAuth(request: Request, config: Config): Promise<AuthContext> {
  const auth = await optionalAuth(request, config);
  if (!auth) throw new HttpError(403, "Missing or invalid bearer token");
  return auth;
}

async function optionalAuth(request: Request, config: Config): Promise<AuthContext | null> {
  const custom = (request.headers.get("x-nola-access-token") || request.headers.get("X-Nola-Access-Token") || "").trim();
  const header = request.headers.get("authorization") || "";
  const token = custom || (header.toLowerCase().startsWith("bearer ") ? header.slice(7).trim() : "");
  if (!token) return null;
  const payload = await verifyJwt(token, config);
  const clientUuid = normalizeUuid(str(payload.client_uuid || payload.user_id || payload.sub));
  return { clientUuid, subject: str(payload.sub) };
}

async function signJwt(payload: JsonObject, config: Config, expiresIn: number): Promise<string> {
  const now = Math.floor(Date.now() / 1000);
  const body = { ...payload, iat: now, ...(expiresIn > 0 ? { exp: now + expiresIn } : {}) };
  const encoded = `${base64UrlJson({ typ: "JWT", alg: "HS256" })}.${base64UrlJson(body)}`;
  const signature = await hmac(encoded, jwtSecret(config));
  return `${encoded}.${base64Url(signature)}`;
}

async function verifyJwt(token: string, config: Config): Promise<JsonObject> {
  const parts = token.split(".");
  if (parts.length !== 3) throw new HttpError(403, "Invalid bearer token");
  const expected = await hmac(`${parts[0]}.${parts[1]}`, jwtSecret(config));
  if (base64Url(expected) !== parts[2]) throw new HttpError(403, "Invalid bearer token");
  const payload = JSON.parse(textFromBase64Url(parts[1])) as JsonObject;
  if (payload.exp && Number(payload.exp) < Math.floor(Date.now() / 1000)) throw new HttpError(403, "Token expired");
  return payload;
}

function mapUser(user: JsonObject): JsonObject {
  return {
    userId: user.user_id,
    email: user.email,
    displayName: user.display_name,
    authProvider: user.auth_provider,
    appleUserId: user.apple_user_id,
    googleUserId: user.google_user_id,
    githubUserId: user.github_user_id,
    emailVerified: Boolean(Number(user.email_verified ?? 0)),
    createdAt: user.created_at,
    updatedAt: user.updated_at,
  };
}

function mapSource(row: JsonObject): JsonObject {
  return {
    sourceId: row.source_id,
    name: row.name,
    url: row.url,
    format: row.format,
    enabled: Boolean(Number(row.enabled ?? 0)),
    scheduleFrequency: row.schedule_frequency,
    scheduledTimeISO: row.scheduled_time_iso,
    scheduledWeekday: row.scheduled_weekday,
    timezone: row.timezone,
    userPrompt: row.user_prompt,
    updatedAt: row.updated_at,
  };
}

function mapResult(row: JsonObject): JsonObject {
  const diagnostics = parseJsonObject<JsonObject>(str(row.diagnostics_json), {});
  const fetchDiagnostics = isObject(diagnostics.fetch) ? diagnostics.fetch : {};
  const failure = {
    reasonCode: diagnostics.reasonCode ?? null,
    reasonMessage: diagnostics.reasonMessage ?? null,
    fetchReason: diagnostics.fetchReason ?? null,
    fetchError: diagnostics.fetchError ?? null,
    llmError: diagnostics.llmError ?? null,
    httpStatus: fetchDiagnostics.httpStatus ?? null,
    contentType: fetchDiagnostics.contentType ?? null,
    bytesRead: fetchDiagnostics.bytesRead ?? null,
    durationMs: fetchDiagnostics.durationMs ?? null,
    url: diagnostics.sourceUrl ?? row.source_url ?? null,
  };
  const failureReason = [failure.reasonMessage, failure.fetchError, failure.llmError, row.llm_explain]
    .map((value) => str(value).trim())
    .find((value) => value.length > 0) || null;
  return {
    resultId: row.result_id,
    jobId: row.job_id,
    clientUuid: row.client_uuid,
    sourceId: row.source_id,
    sourceName: row.source_name ?? null,
    sourceUrl: row.source_url ?? null,
    sourceFormat: row.source_format ?? null,
    scheduledAt: row.scheduled_at,
    fetchedAt: row.fetched_at,
    status: row.status,
    llmSummaryText: row.llm_summary_text,
    llmExplain: row.llm_explain,
    failureReason,
    events: parseJsonObject(str(row.events_json), []),
    createdAt: row.created_at,
    expiresAt: row.expires_at,
    failure,
    ackCount: row.ack_count,
  };
}

function mapResultSummary(row: JsonObject): JsonObject {
  const expiresAt = str(row.expires_at);
  return {
    resultId: row.result_id,
    jobId: row.job_id,
    sourceId: row.source_id,
    sourceName: row.source_name ?? null,
    sourceUrl: row.source_url ?? null,
    sourceFormat: row.source_format ?? null,
    scheduledAt: row.scheduled_at,
    createdAt: row.created_at,
    status: row.status,
    expiresAt: row.expires_at,
    expired: expiresAt ? new Date(expiresAt).getTime() < Date.now() : false,
  };
}

function jsonList(raw: unknown): unknown[] {
  const value = str(raw);
  if (!value) return [];
  try {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function jsonValue(raw: unknown, fallback: unknown = ""): unknown {
  const value = str(raw);
  if (!value) return fallback;
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

async function getOrCreateAgentSettings(db: D1Database, clientUuid: string): Promise<JsonObject> {
  let row = await db.prepare("SELECT * FROM agentsettingrecord WHERE client_uuid = ?").bind(clientUuid).first<JsonObject>();
  if (row) return row;
  const now = new Date().toISOString();
  await db.prepare(
    `INSERT INTO agentsettingrecord (
      client_uuid, enable_local_dev_tools, allowed_dirs_json, bash_prefixes_json,
      agent_llm_endpoint, agent_llm_api_key, agent_llm_model, response_style, long_term_memory, user_preferences,
      mcp_servers_json, uploaded_skills_json, uploaded_documents_json, created_at, updated_at
    ) VALUES (?, 0, '[]', '[]', NULL, NULL, NULL, 'casual', NULL, NULL, '[]', '[]', '[]', ?, ?)`
  ).bind(clientUuid, now, now).run();
  row = await db.prepare("SELECT * FROM agentsettingrecord WHERE client_uuid = ?").bind(clientUuid).first<JsonObject>();
  if (!row) throw new HttpError(500, "Failed to create agent settings");
  return row;
}

function agentSettingsToDict(row: JsonObject): JsonObject {
  return {
    enable_local_dev_tools: Boolean(Number(row.enable_local_dev_tools ?? 0)),
    allowed_dirs: jsonList(row.allowed_dirs_json),
    bash_prefixes: jsonList(row.bash_prefixes_json),
    bash_allowed_prefixes: jsonList(row.bash_prefixes_json),
    agent_llm_endpoint: row.agent_llm_endpoint || "",
    agent_llm_model: row.agent_llm_model || "",
    response_style: row.response_style || "casual",
    long_term_memory: jsonValue(row.long_term_memory, ""),
    user_preferences: jsonValue(row.user_preferences, ""),
    mcp_servers: normalizeAgentSettingItems(jsonList(row.mcp_servers_json), "mcp"),
    uploaded_skills: normalizeAgentSettingItems(jsonList(row.uploaded_skills_json), "skill"),
    uploaded_documents: normalizeAgentSettingItems(jsonList(row.uploaded_documents_json), "document"),
    has_agent_llm_api_key: Boolean(str(row.agent_llm_api_key).trim()),
  };
}

function enabledBool(value: unknown, fallback = true): boolean {
  if (typeof value === "boolean") return value;
  if (value === null || value === undefined) return fallback;
  if (typeof value === "number") {
    if (value === 0) return false;
    if (value === 1) return true;
    return fallback;
  }
  const normalized = str(value).trim().toLowerCase();
  if (["1", "true", "yes", "y", "on"].includes(normalized)) return true;
  if (["", "0", "false", "no", "n", "off"].includes(normalized)) return false;
  return fallback;
}

function normalizeAgentSettingItems(items: unknown[], kind: "mcp" | "skill" | "document"): JsonObject[] {
  return items
    .filter((item): item is JsonObject => item !== null && typeof item === "object" && !Array.isArray(item))
    .map((item) => {
      const out: JsonObject = { ...item };
      out.enabled = enabledBool(out.enabled, true);
      if (kind === "mcp") {
        const endpoint = str(out.endpoint || out.url).trim();
        if (endpoint) {
          out.endpoint = endpoint;
          out.url = str(out.url || endpoint).trim();
        }
      } else {
        const content = str(out.content || out.text).trim();
        if (content) out.content = content;
        if (!str(out.kind).trim()) out.kind = kind === "document" ? "document" : "text";
      }
      return out;
    });
}

function bearerToken(request: Request): string {
  const custom = (request.headers.get("x-nola-access-token") || request.headers.get("X-Nola-Access-Token") || "").trim();
  if (custom) return custom;
  const header = (request.headers.get("authorization") || request.headers.get("Authorization") || "").trim();
  if (!header.toLowerCase().startsWith("bearer ")) throw new HttpError(401, "Missing bearer token");
  const token = header.slice(7).trim();
  if (!token) throw new HttpError(401, "Missing bearer token");
  return token;
}

function isObject(value: unknown): value is JsonObject {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

const MOBILE_AAD = new TextEncoder().encode("nola-mobile-bridge-v1");
const MOBILE_ALG = "AES-256-GCM";
const MOBILE_VERSION = 1;

async function mobileKeyFromToken(token: string): Promise<CryptoKey> {
  if (!token) throw new HttpError(400, "missing access token");
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token));
  return crypto.subtle.importKey("raw", digest, { name: "AES-GCM" }, false, ["encrypt", "decrypt"]);
}

async function encryptMobilePayload(payload: JsonObject, token: string): Promise<JsonObject> {
  const key = await mobileKeyFromToken(token);
  const nonce = crypto.getRandomValues(new Uint8Array(12));
  const plaintext = new TextEncoder().encode(JSON.stringify(payload));
  const sealed = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv: nonce, additionalData: MOBILE_AAD, tagLength: 128 }, key, plaintext));
  const ciphertext = sealed.slice(0, sealed.length - 16);
  const tag = sealed.slice(sealed.length - 16);
  return {
    type: "encrypted",
    v: MOBILE_VERSION,
    alg: MOBILE_ALG,
    nonce: base64Url(nonce),
    ciphertext: base64Url(ciphertext),
    tag: base64Url(tag),
  };
}

async function decryptMobilePayload(envelope: JsonObject, token: string): Promise<JsonObject> {
  if (Number(envelope.v) !== MOBILE_VERSION || str(envelope.alg) !== MOBILE_ALG) throw new HttpError(400, "unsupported encrypted payload");
  const nonce = bytesFromBase64Url(str(envelope.nonce));
  const ciphertext = bytesFromBase64Url(str(envelope.ciphertext));
  const tag = bytesFromBase64Url(str(envelope.tag));
  if (nonce.length !== 12 || tag.length !== 16) throw new HttpError(400, "invalid encrypted payload");
  const key = await mobileKeyFromToken(token);
  try {
    const plaintext = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: nonce, additionalData: MOBILE_AAD, tagLength: 128 },
      key,
      concatBytes(ciphertext, tag),
    );
    const decoded = JSON.parse(new TextDecoder().decode(new Uint8Array(plaintext)));
    if (!isObject(decoded)) throw new HttpError(400, "encrypted payload JSON must be an object");
    return decoded;
  } catch {
    throw new HttpError(400, "encrypted payload authentication failed");
  }
}

function parseAttachmentText(
  raw: Uint8Array,
  filename: string,
  mimeType: string,
): { text: string; parser: "worker_plain_text" | "worker_html_text" } | null {
  const lowerName = filename.toLowerCase();
  const extension = lowerName.includes(".") ? lowerName.split(".").pop() || "" : "";
  const binaryMagic =
    startsWithBytes(raw, [0x25, 0x50, 0x44, 0x46, 0x2d]) || // PDF
    startsWithBytes(raw, [0x50, 0x4b, 0x03, 0x04]) || // ZIP / Open XML
    startsWithBytes(raw, [0xd0, 0xcf, 0x11, 0xe0]); // legacy Office
  const binaryExtension = new Set([
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp",
    "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "tif", "tiff", "bmp",
  ]).has(extension);
  if (binaryMagic || binaryExtension || mimeType.startsWith("image/") || mimeType === "application/pdf") return null;

  const encodings: string[] = startsWithBytes(raw, [0xff, 0xfe])
    ? ["utf-16le"]
    : startsWithBytes(raw, [0xfe, 0xff])
      ? ["utf-16be"]
      : ["utf-8"];
  for (const enc of encodings) {
    try {
      const text = new TextDecoder(enc, { fatal: true, ignoreBOM: false }).decode(raw).replace(/^\uFEFF/, "").trim();
      if (!text || !hasTrustworthyTextContent(text)) continue;
      const html = mimeType.toLowerCase() === "text/html"
        || extension === "html"
        || extension === "htm"
        || /^\s*(?:<!doctype\s+html|<html)[\s>]/i.test(text);
      return {
        text: html ? stripAttachmentHtml(text) : text,
        parser: html ? "worker_html_text" : "worker_plain_text",
      };
    } catch {
      // continue
    }
  }
  return null;
}

function startsWithBytes(raw: Uint8Array, prefix: number[]): boolean {
  return raw.length >= prefix.length && prefix.every((byte, index) => raw[index] === byte);
}

function hasTrustworthyTextContent(text: string): boolean {
  const meaningful = [...text].filter((character) => !/\s/u.test(character));
  if (!meaningful.length) return false;
  const controls = meaningful.filter((character) => {
    const code = character.codePointAt(0) || 0;
    return code < 0x20 || (code >= 0x7f && code <= 0x9f);
  }).length;
  return controls * 100 <= meaningful.length;
}

function stripAttachmentHtml(html: string): string {
  return html
    .replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, " ")
    .replace(/<style\b[^>]*>[\s\S]*?<\/style>/gi, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/&nbsp;/gi, " ")
    .replace(/&amp;/gi, "&")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&quot;/gi, "\"")
    .replace(/&#39;|&apos;/gi, "'")
    .replace(/\s+/g, " ")
    .trim();
}

function normalizePem(value: string): string {
  let out = (value || "").trim();
  if (!out) return "";
  out = out.replace(/\\n/g, "\n");
  if ((out.startsWith('"') && out.endsWith('"')) || (out.startsWith("'") && out.endsWith("'"))) {
    out = out.slice(1, -1).trim();
  }
  return out;
}

function pemToArrayBuffer(pem: string): ArrayBuffer {
  const body = pem
    .replace(/-----BEGIN PRIVATE KEY-----/g, "")
    .replace(/-----END PRIVATE KEY-----/g, "")
    .replace(/\s+/g, "");
  const bytes = bytesFromBase64Url(body.replace(/\+/g, "-").replace(/\//g, "_"));
  const copied = new Uint8Array(bytes.length);
  copied.set(bytes);
  return copied.buffer;
}

async function apnsJwt(config: Config): Promise<string> {
  const teamId = config.get("APNS_TEAM_ID");
  const keyId = config.get("APNS_KEY_ID");
  const key = normalizePem(config.get("APNS_AUTH_KEY_P8"));
  if (!teamId || !keyId || !key) throw new HttpError(500, "APNs auth not configured");
  const now = Math.floor(Date.now() / 1000);
  const header = { alg: "ES256", kid: keyId };
  const claims = { iss: teamId, iat: now };
  const encoded = `${base64UrlJson(header)}.${base64UrlJson(claims)}`;
  const privateKey = await crypto.subtle.importKey("pkcs8", pemToArrayBuffer(key), { name: "ECDSA", namedCurve: "P-256" }, false, ["sign"]);
  const der = new Uint8Array(await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, privateKey, new TextEncoder().encode(encoded)));
  const jose = derToJose(der, 64);
  return `${encoded}.${base64Url(jose)}`;
}

async function sendApns(config: Config, deviceToken: string, payload: JsonObject, pushType: "background" | "alert", priority: string, topicOverride = ""): Promise<JsonObject> {
  const topic = topicOverride || config.get("APNS_TOPIC");
  if (!topic) throw new HttpError(500, "APNS_TOPIC not configured");
  const token = await apnsJwt(config);
  const host = config.bool("APNS_USE_SANDBOX", false) ? "https://api.sandbox.push.apple.com" : "https://api.push.apple.com";
  const response = await fetch(`${host}/3/device/${deviceToken}`, {
    method: "POST",
    headers: {
      authorization: `bearer ${token}`,
      "apns-topic": topic,
      "apns-push-type": pushType,
      "apns-priority": priority,
      "content-type": "application/json",
    },
    body: JSON.stringify(payload),
  });
  const text = await response.text();
  if (response.status !== 200) {
    let reason = "";
    try {
      reason = str((JSON.parse(text) as JsonObject).reason);
    } catch {
      reason = text;
    }
    throw new HttpError(502, `APNs failed: ${response.status} ${reason}`);
  }
  return {
    ok: true,
    accepted: true,
    status: response.status,
    reason: "Accepted",
    apnsId: response.headers.get("apns-id"),
  };
}

function parseJwtHeader(token: string): JsonObject {
  const [head] = token.split(".");
  return JSON.parse(textFromBase64Url(head)) as JsonObject;
}

function parseJwtClaims(token: string): JsonObject {
  const [, claims] = token.split(".");
  return JSON.parse(textFromBase64Url(claims)) as JsonObject;
}

function derToJose(der: Uint8Array, outputLength: number): Uint8Array {
  if (der[0] !== 0x30) throw new Error("Invalid DER signature");
  let offset = 2;
  if (der[1] & 0x80) offset = 2 + (der[1] & 0x7f);
  if (der[offset] !== 0x02) throw new Error("Invalid DER signature");
  const rLength = der[offset + 1];
  const r = der.slice(offset + 2, offset + 2 + rLength);
  const sOffset = offset + 2 + rLength;
  if (der[sOffset] !== 0x02) throw new Error("Invalid DER signature");
  const sLength = der[sOffset + 1];
  const s = der.slice(sOffset + 2, sOffset + 2 + sLength);
  const out = new Uint8Array(outputLength);
  const partLength = outputLength / 2;
  out.set(trimAndPad(r, partLength), 0);
  out.set(trimAndPad(s, partLength), partLength);
  return out;
}

function trimAndPad(input: Uint8Array, size: number): Uint8Array {
  let out = input;
  while (out.length > 0 && out[0] === 0) out = out.slice(1);
  if (out.length > size) out = out.slice(out.length - size);
  const padded = new Uint8Array(size);
  padded.set(out, size - out.length);
  return padded;
}

function concatBytes(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length);
  out.set(a, 0);
  out.set(b, a.length);
  return out;
}

function defaultChatEndpoint(config: Config): string {
  const base = config.get("TARGET_OPENAI_API_BASE", "https://api.openai.com/v1").replace(/\/+$/, "");
  return `${base}/chat/completions`;
}

function upstreamHeaders(config: Config, apiKey: string): HeadersInit {
  const header = config.get("UPSTREAM_AUTH_HEADER", "Authorization");
  const scheme = config.get("UPSTREAM_AUTH_SCHEME", "Bearer");
  return { [header]: scheme ? `${scheme} ${apiKey}` : apiKey };
}

function validateRequestedUuid(value: string): string {
  if (!value.trim()) return crypto.randomUUID();
  return hyphenUuid(value);
}

function normalizeUuid(value: string): string {
  const raw = value.trim().toLowerCase();
  if (!raw) throw new HttpError(400, "Invalid client uuid");
  const compact = raw.replace(/-/g, "");
  if (!/^[0-9a-f]{32}$/.test(compact)) throw new HttpError(400, "Invalid client uuid");
  return compact;
}

function hyphenUuid(value: string): string {
  const compact = normalizeUuid(value);
  return `${compact.slice(0, 8)}-${compact.slice(8, 12)}-${compact.slice(12, 16)}-${compact.slice(16, 20)}-${compact.slice(20)}`;
}

function periodStart(date = new Date()): string {
  return `${date.getUTCFullYear()}-${String(date.getUTCMonth() + 1).padStart(2, "0")}-01`;
}

async function jsonBody(request: Request): Promise<JsonObject> {
  const value = await request.json<unknown>();
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new HttpError(400, "JSON object required");
  return value as JsonObject;
}

function json(data: unknown, status = 200): Response {
  return cors(new Response(JSON.stringify(data), { status, headers: JSON_HEADERS }));
}

function cors(response: Response): Response {
  const headers = new Headers(response.headers);
  // API-only backend: do not permit wildcard browser origins.
  headers.delete("access-control-allow-origin");
  headers.delete("access-control-allow-credentials");
  headers.set("access-control-allow-methods", "GET,POST,PUT,PATCH,DELETE,OPTIONS,HEAD");
  headers.set("access-control-allow-headers", "authorization,content-type,x-client-uuid,x-admin-token,x-debug-token,x-nola-access-token,x-nola-mobile-crypto");
  return new Response(response.body, { status: response.status, statusText: response.statusText, headers });
}

function parseJsonObject<T>(value: string, fallback: T): T {
  try {
    return value ? JSON.parse(value) as T : fallback;
  } catch {
    return fallback;
  }
}

function str(value: unknown): string {
  return typeof value === "string" ? value : value === undefined || value === null ? "" : String(value);
}

function nullable(value: unknown): string | null {
  const text = str(value).trim();
  return text ? text : null;
}

function Booleanish(value: unknown): boolean {
  return value === true || value === "true" || value === "1" || value === 1;
}

function jwtSecret(config: Config): string {
  const secret = config.get("SERVER_JWT_SECRET") || config.get("JWT_SECRET");
  if (!secret) throw new HttpError(500, "SERVER_JWT_SECRET is not configured");
  return secret;
}

async function hmac(data: string, secret: string): Promise<ArrayBuffer> {
  const key = await crypto.subtle.importKey("raw", new TextEncoder().encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return crypto.subtle.sign("HMAC", key, new TextEncoder().encode(data));
}

async function sha256Hex(value: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function hashPassword(password: string): Promise<string> {
  const salt = randomToken();
  const derived = await derivePassword(password, salt);
  return `pbkdf2-sha256$${PASSWORD_HASH_ITERATIONS}$${salt}$${base64Url(derived)}`;
}

async function verifyPassword(password: string, stored: string): Promise<boolean> {
  const parts = stored.split("$");
  if (parts.length !== 4 || parts[0] !== "pbkdf2-sha256") return false;
  const iterations = Number(parts[1]);
  const salt = parts[2] || "";
  const expected = parts[3] || "";
  if (!Number.isFinite(iterations) || iterations < PASSWORD_HASH_ITERATIONS || !salt || !expected) return false;
  const derived = await derivePassword(password, salt, iterations);
  return timingSafeEqual(base64Url(derived), expected);
}

async function derivePassword(password: string, salt: string, iterations = PASSWORD_HASH_ITERATIONS): Promise<ArrayBuffer> {
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(password),
    "PBKDF2",
    false,
    ["deriveBits"],
  );
  return crypto.subtle.deriveBits(
    { name: "PBKDF2", hash: "SHA-256", salt: new TextEncoder().encode(salt), iterations },
    key,
    256,
  );
}

function timingSafeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i += 1) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}

function randomToken(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return base64Url(bytes);
}

function base64UrlJson(value: unknown): string {
  return base64Url(new TextEncoder().encode(JSON.stringify(value)));
}

function base64Url(value: ArrayBuffer | Uint8Array): string {
  const bytes = value instanceof Uint8Array ? value : new Uint8Array(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function bytesFromBase64Url(value: string): Uint8Array {
  const padded = value.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(value.length / 4) * 4, "=");
  const binary = atob(padded);
  return Uint8Array.from(binary, (ch) => ch.charCodeAt(0));
}

function textFromBase64Url(value: string): string {
  return new TextDecoder().decode(bytesFromBase64Url(value));
}
