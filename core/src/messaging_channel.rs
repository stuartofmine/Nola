// SPDX-License-Identifier: Apache-2.0
//! Provider-neutral messaging channel contracts.

use serde::{Deserialize, Serialize};

pub const MESSAGING_CHANNEL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessagingChannelKind {
    Telegram,
    #[serde(rename = "whatsapp")]
    WhatsApp,
    #[serde(rename = "wechat_official")]
    WechatOfficial,
    #[serde(rename = "we_com")]
    WeCom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessagingChannelConnectionState {
    Disabled,
    AwaitingVerification,
    Connected,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagingChannelConnection {
    pub schema_version: u32,
    pub id: String,
    pub kind: MessagingChannelKind,
    pub display_name: String,
    pub state: MessagingChannelConnectionState,
    pub account_id: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_received_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagingChannelEnvelope {
    pub schema_version: u32,
    pub connection_id: String,
    pub kind: MessagingChannelKind,
    pub provider_event_id: String,
    pub external_conversation_id: String,
    pub external_sender_id: String,
    pub received_at: String,
    pub text: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagingChannelLease {
    pub schema_version: u32,
    pub message_id: u64,
    pub lease_id: String,
    pub lease_expires_at: String,
    pub envelope: MessagingChannelEnvelope,
    #[serde(default)]
    pub attempt_count: u32,
}

impl MessagingChannelLease {
    pub fn is_valid(&self) -> bool {
        self.schema_version == MESSAGING_CHANNEL_SCHEMA_VERSION
            && self.message_id > 0
            && !self.lease_id.trim().is_empty()
            && !self.lease_expires_at.trim().is_empty()
            && self.envelope.is_valid()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagingChannelCompletion {
    pub schema_version: u32,
    pub message_id: u64,
    pub lease_id: String,
    pub reply_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nola_conversation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagingChannelFailure {
    pub schema_version: u32,
    pub message_id: u64,
    pub lease_id: String,
    pub error: String,
    #[serde(default = "default_true")]
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagingChannelLeaseRenewal {
    pub schema_version: u32,
    pub message_id: u64,
    pub lease_id: String,
    pub lease_seconds: u32,
}

impl MessagingChannelLeaseRenewal {
    pub fn is_valid(&self) -> bool {
        self.schema_version == MESSAGING_CHANNEL_SCHEMA_VERSION
            && self.message_id > 0
            && !self.lease_id.trim().is_empty()
            && (30..=900).contains(&self.lease_seconds)
    }
}

fn default_true() -> bool {
    true
}

impl MessagingChannelFailure {
    pub fn is_valid(&self) -> bool {
        self.schema_version == MESSAGING_CHANNEL_SCHEMA_VERSION
            && self.message_id > 0
            && !self.lease_id.trim().is_empty()
            && !self.error.trim().is_empty()
    }
}

impl MessagingChannelCompletion {
    pub fn is_valid(&self) -> bool {
        self.schema_version == MESSAGING_CHANNEL_SCHEMA_VERSION
            && self.message_id > 0
            && !self.lease_id.trim().is_empty()
            && !self.reply_text.trim().is_empty()
    }
}

impl MessagingChannelEnvelope {
    pub fn idempotency_key(&self) -> String {
        format!("{}:{}", self.connection_id, self.provider_event_id)
    }

    pub fn is_valid(&self) -> bool {
        self.schema_version == MESSAGING_CHANNEL_SCHEMA_VERSION
            && !self.connection_id.trim().is_empty()
            && !self.provider_event_id.trim().is_empty()
            && !self.external_conversation_id.trim().is_empty()
            && !self.external_sender_id.trim().is_empty()
            && (!self.text.trim().is_empty() || !self.attachment_ids.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_retry_has_a_stable_connection_scoped_key() {
        let envelope = MessagingChannelEnvelope {
            schema_version: MESSAGING_CHANNEL_SCHEMA_VERSION,
            connection_id: "telegram-primary".into(),
            kind: MessagingChannelKind::Telegram,
            provider_event_id: "42".into(),
            external_conversation_id: "100".into(),
            external_sender_id: "7".into(),
            received_at: "2026-08-12T00:00:00Z".into(),
            text: "hello".into(),
            attachment_ids: vec![],
        };
        assert!(envelope.is_valid());
        assert_eq!(envelope.idempotency_key(), "telegram-primary:42");
    }

    #[test]
    fn leased_message_and_completion_preserve_provider_identity() {
        let envelope = MessagingChannelEnvelope {
            schema_version: MESSAGING_CHANNEL_SCHEMA_VERSION,
            connection_id: "whatsapp-primary".into(),
            kind: MessagingChannelKind::WhatsApp,
            provider_event_id: "wamid.1".into(),
            external_conversation_id: "8613800000000".into(),
            external_sender_id: "8613800000000".into(),
            received_at: "2026-08-13T00:00:00Z".into(),
            text: "summarize my day".into(),
            attachment_ids: vec![],
        };
        let lease = MessagingChannelLease {
            schema_version: MESSAGING_CHANNEL_SCHEMA_VERSION,
            message_id: 9,
            lease_id: "lease-9".into(),
            lease_expires_at: "2026-08-13T00:03:00Z".into(),
            envelope,
            attempt_count: 1,
        };
        assert!(lease.is_valid());
        let encoded = serde_json::to_string(&lease).expect("serialize lease");
        let decoded: MessagingChannelLease = serde_json::from_str(&encoded).expect("decode lease");
        assert_eq!(
            decoded.envelope.idempotency_key(),
            "whatsapp-primary:wamid.1"
        );
        assert!(MessagingChannelCompletion {
            schema_version: MESSAGING_CHANNEL_SCHEMA_VERSION,
            message_id: decoded.message_id,
            lease_id: decoded.lease_id,
            reply_text: "Here is your summary".into(),
            nola_conversation_id: Some("channel:whatsapp-primary:8613800000000".into()),
        }
        .is_valid());
        let failure: MessagingChannelFailure = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "message_id": 9,
            "lease_id": "lease-9",
            "error": "provider timeout"
        }))
        .expect("decode failure");
        assert!(failure.retryable);
        assert!(failure.is_valid());
        assert!(MessagingChannelLeaseRenewal {
            schema_version: 1,
            message_id: 9,
            lease_id: "lease-9".into(),
            lease_seconds: 180,
        }
        .is_valid());
    }

    #[test]
    fn every_official_gateway_channel_has_a_stable_wire_identifier() {
        assert_eq!(
            serde_json::to_string(&MessagingChannelKind::Telegram).unwrap(),
            "\"telegram\""
        );
        assert_eq!(
            serde_json::to_string(&MessagingChannelKind::WhatsApp).unwrap(),
            "\"whatsapp\""
        );
        assert_eq!(
            serde_json::to_string(&MessagingChannelKind::WechatOfficial).unwrap(),
            "\"wechat_official\""
        );
        assert_eq!(
            serde_json::to_string(&MessagingChannelKind::WeCom).unwrap(),
            "\"we_com\""
        );
    }
}
