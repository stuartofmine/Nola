// SPDX-License-Identifier: Apache-2.0
//! Transported transcript source claims are history, never execution authority.
use crate::{CanonicalBodyReference,CanonicalPartOwner};
use serde::{Deserialize,Serialize};

#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum ToolSourceKind { ToolCall,ToolResult }

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSourceOriginalOwner {
    pub part_owner:CanonicalPartOwner,
    pub run_id:String,
    pub turn_id:String,
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSourceDerivationClaim {
    pub operation_id:String,
    pub source_owner:CanonicalPartOwner,
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSourceClaim {
    pub schema_version:u32,
    pub source_item_id:String,
    pub kind:ToolSourceKind,
    pub call_id:String,
    pub name:String,
    pub owner:CanonicalPartOwner,
    pub original_owner:ToolSourceOriginalOwner,
    pub body:CanonicalBodyReference,
    #[serde(deserialize_with="required_derivation")]
    pub derivation:Option<ToolSourceDerivationClaim>,
}
fn required_derivation<'de,D:serde::Deserializer<'de>>(deserializer:D)->Result<Option<ToolSourceDerivationClaim>,D::Error> {
    Option::<ToolSourceDerivationClaim>::deserialize(deserializer)
}
impl ToolSourceClaim {
    pub fn validate(&self)->Result<(),String> {
        let valid=|value:&str,maximum:usize| !value.is_empty() && value.trim()==value
            && value.len()<=maximum && !value.chars().any(char::is_control);
        if self.schema_version!=1 || !valid(&self.source_item_id,128) || !valid(&self.call_id,512)
            || !valid(&self.name,512) || !valid(&self.original_owner.run_id,128) || !valid(&self.original_owner.turn_id,128)
            || self.body.conversation_id()!=self.owner.conversation_id()
            || self.body.message_id()!=self.owner.message_id() || self.body.part_id()!=self.owner.part_id()
            || self.body.field()!=crate::PartBodyField::ContentJson || self.body.total_utf8_bytes()>256*1024*1024
            || self.derivation.as_ref().is_some_and(|d|!valid(&d.operation_id,128))
        {return Err("invalid current tool source claim identity, body or derivation".into());}
        Ok(())
    }
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSourceExportRange {
    pub owner:CanonicalPartOwner,
    pub source_item_id:String,
    pub revision:i64,
    pub offset_bytes:u64,
    pub next_offset_bytes:u64,
    pub total_utf8_bytes:u64,
    pub text:String,
    pub end_of_content:bool,
}
