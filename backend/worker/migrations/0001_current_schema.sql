CREATE TABLE IF NOT EXISTS useraccountrecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id TEXT NOT NULL UNIQUE,
  email TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL DEFAULT '',
  display_name TEXT,
  auth_provider TEXT NOT NULL DEFAULT 'apple',
  apple_user_id TEXT,
  google_user_id TEXT,
  github_user_id TEXT,
  email_verified INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_user_apple_id ON useraccountrecord(apple_user_id) WHERE apple_user_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS ux_user_google_id ON useraccountrecord(google_user_id) WHERE google_user_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS ux_user_github_id ON useraccountrecord(github_user_id) WHERE github_user_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS useridentityrecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  client_uuid TEXT NOT NULL,
  apple_user_id TEXT NOT NULL,
  home_region TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS ix_useridentity_client ON useridentityrecord(client_uuid);

CREATE TABLE IF NOT EXISTS oauthrelaystaterecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  state_hash TEXT NOT NULL UNIQUE,
  provider TEXT NOT NULL,
  local_redirect_uri TEXT NOT NULL,
  local_state TEXT NOT NULL,
  client_uuid TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  consumed_at TEXT
);

CREATE TABLE IF NOT EXISTS oauthrelaycoderecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  code_hash TEXT NOT NULL UNIQUE,
  payload_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  consumed_at TEXT
);

CREATE TABLE IF NOT EXISTS pushdevicerecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  device_id TEXT NOT NULL UNIQUE,
  client_uuid TEXT NOT NULL,
  device_token TEXT NOT NULL,
  platform TEXT NOT NULL DEFAULT 'ios',
  timezone TEXT NOT NULL DEFAULT 'UTC',
  push_enabled INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(client_uuid, device_token)
);

CREATE TABLE IF NOT EXISTS scheduledsourcerecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  client_uuid TEXT NOT NULL,
  source_id TEXT NOT NULL,
  name TEXT NOT NULL,
  url TEXT NOT NULL,
  format TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  schedule_frequency TEXT NOT NULL DEFAULT 'none',
  scheduled_time_iso TEXT,
  scheduled_weekday INTEGER,
  timezone TEXT NOT NULL DEFAULT 'UTC',
  user_prompt TEXT,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(client_uuid, source_id)
);

CREATE TABLE IF NOT EXISTS scheduledjobrecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  job_id TEXT NOT NULL UNIQUE,
  client_uuid TEXT NOT NULL,
  source_id TEXT NOT NULL,
  scheduled_at TEXT NOT NULL,
  run_at TEXT,
  status TEXT NOT NULL DEFAULT 'queued',
  result_id TEXT,
  attempt INTEGER NOT NULL DEFAULT 0,
  error_message TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS scheduledresultrecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  result_id TEXT NOT NULL UNIQUE,
  job_id TEXT NOT NULL,
  client_uuid TEXT NOT NULL,
  source_id TEXT NOT NULL,
  scheduled_at TEXT NOT NULL,
  fetched_at TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'success',
  llm_summary_text TEXT NOT NULL,
  llm_explain TEXT,
  events_json TEXT NOT NULL DEFAULT '[]',
  diagnostics_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  expires_at TEXT NOT NULL,
  visible_fallback_sent INTEGER NOT NULL DEFAULT 0,
  fallback_due_at TEXT,
  ack_count INTEGER NOT NULL DEFAULT 0,
  silent_push_sent INTEGER NOT NULL DEFAULT 0,
  silent_push_sent_at TEXT
);

CREATE TABLE IF NOT EXISTS scheduledresultackrecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  result_id TEXT NOT NULL,
  client_uuid TEXT NOT NULL,
  device_id TEXT NOT NULL,
  received_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  status TEXT NOT NULL DEFAULT 'pulled_and_persisted',
  UNIQUE(result_id, device_id)
);

CREATE TABLE IF NOT EXISTS notificationrecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  notification_id TEXT NOT NULL UNIQUE,
  client_uuid TEXT NOT NULL,
  title TEXT NOT NULL,
  body TEXT,
  source TEXT NOT NULL DEFAULT 'system',
  notification_type TEXT NOT NULL DEFAULT 'message',
  dedupe_key TEXT NOT NULL,
  payload_json TEXT NOT NULL DEFAULT '{}',
  is_read INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  read_at TEXT,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(client_uuid, dedupe_key)
);

CREATE INDEX IF NOT EXISTS ix_notification_client_created ON notificationrecord(client_uuid, created_at);
CREATE INDEX IF NOT EXISTS ix_notification_client_unread ON notificationrecord(client_uuid, is_read);

CREATE TABLE IF NOT EXISTS chatconversationrecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  conversation_id TEXT NOT NULL UNIQUE,
  client_uuid TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '新对话',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS chatmessagerecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  conversation_id TEXT NOT NULL,
  client_uuid TEXT NOT NULL,
  role TEXT NOT NULL,
  content TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS agentsettingrecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  client_uuid TEXT NOT NULL UNIQUE,
  enable_local_dev_tools INTEGER NOT NULL DEFAULT 0,
  allowed_dirs_json TEXT NOT NULL DEFAULT '[]',
  bash_prefixes_json TEXT NOT NULL DEFAULT '[]',
  agent_llm_endpoint TEXT,
  agent_llm_api_key TEXT,
  agent_llm_model TEXT,
  long_term_memory TEXT,
  user_preferences TEXT,
  mcp_servers_json TEXT NOT NULL DEFAULT '[]',
  uploaded_skills_json TEXT NOT NULL DEFAULT '[]',
  uploaded_documents_json TEXT NOT NULL DEFAULT '[]',
  response_style TEXT NOT NULL DEFAULT 'casual',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS agentpermissionrulerecord (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  client_uuid TEXT NOT NULL,
  tool_name TEXT NOT NULL,
  behavior TEXT NOT NULL,
  match TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_agent_permission_rule
  ON agentpermissionrulerecord(client_uuid, tool_name, behavior, match);
CREATE TABLE IF NOT EXISTS canonicalreplicachangerecord (
  change_cursor INTEGER PRIMARY KEY AUTOINCREMENT,
  client_uuid TEXT NOT NULL,
  operation_id TEXT NOT NULL,
  entity_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  revision INTEGER NOT NULL,
  deleted INTEGER NOT NULL DEFAULT 0,
  payload_json TEXT NOT NULL DEFAULT 'null',
  origin_device_id TEXT NOT NULL,
  committed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(client_uuid, operation_id),
  UNIQUE(client_uuid, entity_type, entity_id, revision)
);

CREATE INDEX IF NOT EXISTS ix_canonical_replica_client_cursor
  ON canonicalreplicachangerecord(client_uuid, change_cursor);

CREATE INDEX IF NOT EXISTS ix_canonical_replica_entity_revision
  ON canonicalreplicachangerecord(client_uuid, entity_type, entity_id, revision DESC);
CREATE TABLE IF NOT EXISTS messagingchannelconnectionrecord (
    connection_id TEXT PRIMARY KEY,
    client_uuid TEXT NOT NULL,
    kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    account_id TEXT NOT NULL DEFAULT '',
    secret_ref TEXT NOT NULL,
    credential_ref TEXT NOT NULL DEFAULT '',
    webhook_secret_ref TEXT NOT NULL DEFAULT '',
    verify_token_ref TEXT NOT NULL DEFAULT '',
    provider_config_json TEXT NOT NULL DEFAULT '{}',
    enabled INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'awaiting_verification',
    last_received_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_messaging_channel_owner
ON messagingchannelconnectionrecord(client_uuid, updated_at DESC);

CREATE TABLE IF NOT EXISTS messagingchannelmessagerecord (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_id TEXT NOT NULL,
    provider_event_id TEXT NOT NULL,
    external_conversation_id TEXT NOT NULL,
    external_sender_id TEXT NOT NULL,
    direction TEXT NOT NULL DEFAULT 'inbound',
    status TEXT NOT NULL DEFAULT 'received',
    text TEXT NOT NULL DEFAULT '',
    payload_json TEXT NOT NULL DEFAULT '{}',
    nola_conversation_id TEXT,
    received_at TEXT NOT NULL,
    processed_at TEXT,
    lease_id TEXT,
    lease_expires_at TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    reply_text TEXT NOT NULL DEFAULT '',
    provider_reply_id TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(connection_id, provider_event_id)
);

CREATE INDEX IF NOT EXISTS idx_messaging_channel_inbox
ON messagingchannelmessagerecord(connection_id, status, id);

CREATE INDEX IF NOT EXISTS idx_messaging_channel_claim
ON messagingchannelmessagerecord(status, lease_expires_at, id);
