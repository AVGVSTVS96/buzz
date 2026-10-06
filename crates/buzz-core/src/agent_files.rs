//! NIP-AF Agent Files — pure crypto + parsing primitives.
//!
//! See `docs/nips/NIP-AF.md` for the spec. A companion to NIP-AE: records are
//! encrypted under the same agent ↔ owner conversation key and addressed by
//! the same HMAC construction, under their own domain. This module is
//! I/O-free: it does not talk to relays or filesystems.
//!
//! Shared by `buzz-acp` (publishing shared files, answering edit requests)
//! and the desktop (listing files, proposing edits).

use nostr::nips::nip44::{self, Version};
use nostr::{Event, EventBuilder, EventId, Keys, Kind, PublicKey, SecretKey, Tag};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use crate::engram::{conversation_key, monotonic_created_at, select_head, NIP44_PLAINTEXT_MAX};
use crate::engram::{domain_d_tag, parse_strict_json, write_json_string};
use crate::kind::{KIND_AGENT_FILE, KIND_AGENT_FILE_EDIT_REQUEST, KIND_AGENT_FILE_EDIT_RESULT};

/// Domain prefix for the `d`-tag HMAC. Followed by a `0x00` byte and the path.
/// Versioned independently of the NIP number; future revisions MUST change it.
pub const D_TAG_DOMAIN: &[u8] = b"agent-files/v1/d-tag";

/// Maximum path length in bytes (spec: *Paths*).
pub const PATH_MAX_LEN: usize = 255;

/// Errors from agent-file operations.
#[derive(Debug, thiserror::Error)]
pub enum AgentFileError {
    /// Path failed the *Paths* grammar.
    #[error("invalid path: {0}")]
    InvalidPath(String),
    /// Body parsing or shape check failed.
    #[error("invalid body: {0}")]
    InvalidBody(String),
    /// Event failed the envelope rules — kind, author, tag shape, addressing.
    #[error("invalid envelope: {0}")]
    InvalidEnvelope(String),
    /// NIP-44 decryption failed.
    #[error("decrypt failed")]
    Decrypt,
    /// Encryption failed.
    #[error("encrypt failed: {0}")]
    Encrypt(String),
    /// Body exceeds the NIP-44 plaintext cap.
    #[error("body exceeds {NIP44_PLAINTEXT_MAX}-byte plaintext limit ({0} bytes)")]
    BodyTooLarge(usize),
    /// Signing error.
    #[error("sign failed: {0}")]
    Sign(String),
}

/// Validate a path against the *Paths* grammar: relative, `/`-separated,
/// at most 255 bytes, no empty, `.` or `..` segments, and no `\` or control
/// characters.
pub fn validate_path(path: &str) -> Result<(), AgentFileError> {
    if path.is_empty() {
        return Err(AgentFileError::InvalidPath("empty".into()));
    }
    if path.len() > PATH_MAX_LEN {
        return Err(AgentFileError::InvalidPath(format!(
            "length {} exceeds {}",
            path.len(),
            PATH_MAX_LEN
        )));
    }
    if let Some(c) = path.chars().find(|&c| c == '\\' || c.is_control()) {
        return Err(AgentFileError::InvalidPath(format!(
            "{path:?} contains {c:?}"
        )));
    }
    for (i, segment) in path.split('/').enumerate() {
        if matches!(segment, "" | "." | "..") {
            return Err(AgentFileError::InvalidPath(format!(
                "segment {} of {path:?} is {segment:?}",
                i + 1
            )));
        }
    }
    Ok(())
}

/// Compute the `d` tag for a path under a conversation key.
///
/// `d = lower_hex(HMAC-SHA256(K_c, "agent-files/v1/d-tag" || 0x00 || path))`,
/// 64 hex characters.
pub fn d_tag(k_c: &nip44::v2::ConversationKey, path: &str) -> String {
    domain_d_tag(k_c, D_TAG_DOMAIN, path)
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// A decoded file record body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// A shared file. `content` is `None` when the file is not UTF-8 text or
    /// its body would exceed the NIP-44 plaintext limit — listed, not inlined.
    File {
        /// The file's path, relative to the agent's share.
        path: String,
        /// Lowercase hex SHA-256 of the file's bytes.
        sha256: String,
        /// The file's size in bytes.
        size: u64,
        /// The file's text, when inlined.
        content: Option<String>,
    },
    /// Tombstone — the file was removed or is no longer shared.
    Removed {
        /// The path that is no longer shared.
        path: String,
    },
}

impl Body {
    /// Describe a file's bytes, inlining them as `content` when they are
    /// UTF-8 and the serialized body fits the NIP-44 plaintext limit.
    pub fn for_contents(path: String, bytes: &[u8]) -> Self {
        let sha256 = sha256_hex(bytes);
        let size = bytes.len() as u64;
        if let Ok(text) = std::str::from_utf8(bytes) {
            let inlined = Body::File {
                path: path.clone(),
                sha256: sha256.clone(),
                size,
                content: Some(text.to_string()),
            };
            if inlined.to_json_bytes().len() <= NIP44_PLAINTEXT_MAX {
                return inlined;
            }
        }
        Body::File {
            path,
            sha256,
            size,
            content: None,
        }
    }

    /// Return the path this body addresses.
    pub fn path(&self) -> &str {
        match self {
            Body::File { path, .. } | Body::Removed { path } => path,
        }
    }

    /// `true` if this is a tombstone.
    pub fn is_tombstone(&self) -> bool {
        matches!(self, Body::Removed { .. })
    }

    /// Serialize to the exact JSON encoding the spec specifies for the body
    /// passed to NIP-44. Whitespace-free, members in spec order.
    pub fn to_json_bytes(&self) -> Vec<u8> {
        let mut out = String::with_capacity(96);
        out.push_str("{\"path\":");
        write_json_string(&mut out, self.path());
        match self {
            Body::File {
                sha256,
                size,
                content,
                ..
            } => {
                out.push_str(",\"sha256\":");
                write_json_string(&mut out, sha256);
                out.push_str(&format!(",\"size\":{size}"));
                if let Some(content) = content {
                    out.push_str(",\"content\":");
                    write_json_string(&mut out, content);
                }
            }
            Body::Removed { .. } => out.push_str(",\"removed\":true"),
        }
        out.push('}');
        out.into_bytes()
    }

    /// Parse a body from its decrypted JSON bytes. Rejects duplicate object
    /// member names and inlined content that does not match `sha256`/`size`.
    /// Unknown fields are ignored.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, AgentFileError> {
        let obj = parse_object(bytes)?;
        let path = path_field(&obj)?;
        match obj.get("removed") {
            Some(serde_json::Value::Bool(true)) => return Ok(Body::Removed { path }),
            Some(_) => {
                return Err(AgentFileError::InvalidBody(
                    "`removed` is not `true`".into(),
                ))
            }
            None => {}
        }
        let sha256 = sha256_field(&obj, "sha256")?
            .ok_or_else(|| AgentFileError::InvalidBody("missing `sha256`".into()))?;
        let size = match obj.get("size") {
            Some(v) => v
                .as_u64()
                .ok_or_else(|| AgentFileError::InvalidBody("`size` is not a u64".into()))?,
            None => return Err(AgentFileError::InvalidBody("missing `size`".into())),
        };
        let content = string_field(&obj, "content")?;
        if let Some(text) = &content {
            if text.len() as u64 != size || sha256_hex(text.as_bytes()) != sha256 {
                return Err(AgentFileError::InvalidBody(
                    "`content` does not match `sha256` and `size`".into(),
                ));
            }
        }
        Ok(Body::File {
            path,
            sha256,
            size,
            content,
        })
    }
}

/// An owner's proposed replacement for a shared file (`kind:4180`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRequest {
    /// The file to replace.
    pub path: String,
    /// SHA-256 of the version the owner edited.
    pub base_sha256: String,
    /// The proposed full UTF-8 content.
    pub content: String,
}

impl EditRequest {
    /// Serialize to the spec's JSON encoding.
    pub fn to_json_bytes(&self) -> Vec<u8> {
        let mut out = String::with_capacity(96 + self.content.len());
        out.push_str("{\"path\":");
        write_json_string(&mut out, &self.path);
        out.push_str(",\"base_sha256\":");
        write_json_string(&mut out, &self.base_sha256);
        out.push_str(",\"content\":");
        write_json_string(&mut out, &self.content);
        out.push('}');
        out.into_bytes()
    }

    /// Parse and shape-check a decrypted request body.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, AgentFileError> {
        let obj = parse_object(bytes)?;
        Ok(EditRequest {
            path: path_field(&obj)?,
            base_sha256: sha256_field(&obj, "base_sha256")?
                .ok_or_else(|| AgentFileError::InvalidBody("missing `base_sha256`".into()))?,
            content: string_field(&obj, "content")?
                .ok_or_else(|| AgentFileError::InvalidBody("missing `content`".into()))?,
        })
    }
}

/// How the agent answered an edit request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditStatus {
    /// The agent wrote the proposed content.
    Applied,
    /// The file changed since the owner's base version.
    Conflict,
    /// The agent chose not to apply the edit.
    Declined,
}

impl EditStatus {
    fn as_str(self) -> &'static str {
        match self {
            EditStatus::Applied => "applied",
            EditStatus::Conflict => "conflict",
            EditStatus::Declined => "declined",
        }
    }
}

/// The agent's answer to an edit request (`kind:4181`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditResult {
    /// Outcome.
    pub status: EditStatus,
    /// The path named by the request.
    pub path: String,
    /// The file's SHA-256 after the answer: the new version when applied,
    /// the agent's current version on conflict. Required when applied.
    pub sha256: Option<String>,
    /// Human-readable explanation, typically for `declined`.
    pub reason: Option<String>,
}

impl EditResult {
    /// Serialize to the spec's JSON encoding.
    pub fn to_json_bytes(&self) -> Vec<u8> {
        let mut out = String::with_capacity(128);
        out.push_str("{\"status\":");
        write_json_string(&mut out, self.status.as_str());
        out.push_str(",\"path\":");
        write_json_string(&mut out, &self.path);
        if let Some(sha256) = &self.sha256 {
            out.push_str(",\"sha256\":");
            write_json_string(&mut out, sha256);
        }
        if let Some(reason) = &self.reason {
            out.push_str(",\"reason\":");
            write_json_string(&mut out, reason);
        }
        out.push('}');
        out.into_bytes()
    }

    /// Parse and shape-check a decrypted result body.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, AgentFileError> {
        let obj = parse_object(bytes)?;
        let status = match obj.get("status").and_then(|v| v.as_str()) {
            Some("applied") => EditStatus::Applied,
            Some("conflict") => EditStatus::Conflict,
            Some("declined") => EditStatus::Declined,
            _ => {
                return Err(AgentFileError::InvalidBody(
                    "`status` is not applied, conflict or declined".into(),
                ))
            }
        };
        let sha256 = sha256_field(&obj, "sha256")?;
        if status == EditStatus::Applied && sha256.is_none() {
            return Err(AgentFileError::InvalidBody(
                "applied missing `sha256`".into(),
            ));
        }
        Ok(EditResult {
            status,
            path: path_field(&obj)?,
            sha256,
            reason: string_field(&obj, "reason")?,
        })
    }
}

type JsonObject = serde_json::Map<String, serde_json::Value>;

fn parse_object(bytes: &[u8]) -> Result<JsonObject, AgentFileError> {
    match parse_strict_json(bytes).map_err(AgentFileError::InvalidBody)? {
        serde_json::Value::Object(obj) => Ok(obj),
        _ => Err(AgentFileError::InvalidBody(
            "top-level not an object".into(),
        )),
    }
}

fn string_field(obj: &JsonObject, name: &str) -> Result<Option<String>, AgentFileError> {
    match obj.get(name) {
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(AgentFileError::InvalidBody(format!(
            "`{name}` is not a string"
        ))),
        None => Ok(None),
    }
}

fn path_field(obj: &JsonObject) -> Result<String, AgentFileError> {
    let path = string_field(obj, "path")?
        .ok_or_else(|| AgentFileError::InvalidBody("missing `path`".into()))?;
    validate_path(&path)?;
    Ok(path)
}

fn sha256_field(obj: &JsonObject, name: &str) -> Result<Option<String>, AgentFileError> {
    match string_field(obj, name)? {
        Some(s) if !is_sha256_hex(&s) => Err(AgentFileError::InvalidBody(format!(
            "`{name}` must be 64 lowercase hex chars"
        ))),
        other => Ok(other),
    }
}

/// Build a signed `kind:30180` file record.
///
/// * `created_at` is the timestamp to sign — callers MUST supply a value
///   respecting the *Writing* monotonic rule (`max(now, T_head + 1)`).
/// * Returns `BodyTooLarge` if the serialized body exceeds 65,535 bytes;
///   [`Body::for_contents`] never produces such a body.
pub fn build_file_event(
    agent_keys: &Keys,
    owner_pubkey: &PublicKey,
    body: &Body,
    created_at: u64,
) -> Result<Event, AgentFileError> {
    let k_c = conversation_key(agent_keys.secret_key(), owner_pubkey);
    let tags = [
        ["d".to_string(), d_tag(&k_c, body.path())],
        ["p".to_string(), owner_pubkey.to_hex()],
    ];
    seal(
        agent_keys,
        owner_pubkey,
        KIND_AGENT_FILE,
        &body.to_json_bytes(),
        &tags,
        Some(created_at),
    )
}

/// Validate a `kind:30180` event against *Head selection* and return the
/// decoded body. Caller must verify the signature beforehand.
///
/// * `expected_agent` — the agent pubkey the event's `pubkey` field must equal.
/// * `expected_owner` — the owner pubkey the event's `p` tag must contain.
/// * `my_seckey` / `their_pubkey` — the NIP-44 ECDH pair the caller holds:
///   `(seckey_a, pubkey_o)` for the agent, `(seckey_o, pubkey_a)` for the
///   owner. Either yields the same `K_c`.
pub fn validate_and_decrypt(
    event: &Event,
    expected_agent: &PublicKey,
    expected_owner: &PublicKey,
    my_seckey: &SecretKey,
    their_pubkey: &PublicKey,
) -> Result<Body, AgentFileError> {
    check_envelope(event, KIND_AGENT_FILE, expected_agent, expected_owner)?;
    let d = single_hex_tag(event, "d")?;
    let body = Body::from_json_bytes(&open(event, my_seckey, their_pubkey)?)?;
    let k_c = conversation_key(my_seckey, their_pubkey);
    if d_tag(&k_c, body.path()) != d {
        return Err(AgentFileError::InvalidEnvelope(
            "body path does not re-derive to d tag".into(),
        ));
    }
    Ok(body)
}

/// Build a signed `kind:4180` edit request from the owner to the agent.
pub fn build_edit_request(
    owner_keys: &Keys,
    agent_pubkey: &PublicKey,
    request: &EditRequest,
) -> Result<Event, AgentFileError> {
    validate_path(&request.path)?;
    if !is_sha256_hex(&request.base_sha256) {
        return Err(AgentFileError::InvalidBody(
            "`base_sha256` must be 64 lowercase hex chars".into(),
        ));
    }
    let tags = [["p".to_string(), agent_pubkey.to_hex()]];
    seal(
        owner_keys,
        agent_pubkey,
        KIND_AGENT_FILE_EDIT_REQUEST,
        &request.to_json_bytes(),
        &tags,
        None,
    )
}

/// Validate and decrypt a `kind:4180` edit request authored by
/// `expected_owner` and addressed to `expected_agent`. Caller must verify the
/// signature beforehand.
pub fn decrypt_edit_request(
    event: &Event,
    expected_owner: &PublicKey,
    expected_agent: &PublicKey,
    my_seckey: &SecretKey,
    their_pubkey: &PublicKey,
) -> Result<EditRequest, AgentFileError> {
    check_envelope(
        event,
        KIND_AGENT_FILE_EDIT_REQUEST,
        expected_owner,
        expected_agent,
    )?;
    EditRequest::from_json_bytes(&open(event, my_seckey, their_pubkey)?)
}

/// Build a signed `kind:4181` edit result answering `request_id`.
pub fn build_edit_result(
    agent_keys: &Keys,
    owner_pubkey: &PublicKey,
    request_id: &EventId,
    result: &EditResult,
) -> Result<Event, AgentFileError> {
    let tags = [
        ["p".to_string(), owner_pubkey.to_hex()],
        ["e".to_string(), request_id.to_hex()],
    ];
    seal(
        agent_keys,
        owner_pubkey,
        KIND_AGENT_FILE_EDIT_RESULT,
        &result.to_json_bytes(),
        &tags,
        None,
    )
}

/// Validate and decrypt a `kind:4181` edit result authored by
/// `expected_agent` and addressed to `expected_owner`. Returns the id of the
/// request it answers. Caller must verify the signature beforehand.
pub fn decrypt_edit_result(
    event: &Event,
    expected_agent: &PublicKey,
    expected_owner: &PublicKey,
    my_seckey: &SecretKey,
    their_pubkey: &PublicKey,
) -> Result<(EventId, EditResult), AgentFileError> {
    check_envelope(
        event,
        KIND_AGENT_FILE_EDIT_RESULT,
        expected_agent,
        expected_owner,
    )?;
    let request_id = EventId::from_hex(&single_hex_tag(event, "e")?)
        .map_err(|e| AgentFileError::InvalidEnvelope(e.to_string()))?;
    let result = EditResult::from_json_bytes(&open(event, my_seckey, their_pubkey)?)?;
    Ok((request_id, result))
}

fn seal(
    keys: &Keys,
    counterparty: &PublicKey,
    kind: u32,
    plaintext: &[u8],
    tags: &[[String; 2]],
    created_at: Option<u64>,
) -> Result<Event, AgentFileError> {
    if plaintext.len() > NIP44_PLAINTEXT_MAX {
        return Err(AgentFileError::BodyTooLarge(plaintext.len()));
    }
    let plaintext = std::str::from_utf8(plaintext)
        .map_err(|e| AgentFileError::Encrypt(format!("body JSON not UTF-8: {e}")))?;
    let ciphertext = nip44::encrypt(keys.secret_key(), counterparty, plaintext, Version::V2)
        .map_err(|e| AgentFileError::Encrypt(e.to_string()))?;
    let tags = tags
        .iter()
        .map(|tag| Tag::parse(tag).map_err(|e| AgentFileError::Encrypt(e.to_string())))
        .collect::<Result<Vec<_>, _>>()?;
    let mut builder = EventBuilder::new(Kind::Custom(kind as u16), ciphertext).tags(tags);
    if let Some(created_at) = created_at {
        builder = builder.custom_created_at(nostr::Timestamp::from(created_at));
    }
    builder
        .sign_with_keys(keys)
        .map_err(|e| AgentFileError::Sign(e.to_string()))
}

fn open(
    event: &Event,
    my_seckey: &SecretKey,
    their_pubkey: &PublicKey,
) -> Result<Vec<u8>, AgentFileError> {
    nip44::decrypt(my_seckey, their_pubkey, &event.content)
        .map(String::into_bytes)
        .map_err(|_| AgentFileError::Decrypt)
}

/// Kind, author, and exactly one `p` tag naming `recipient`.
fn check_envelope(
    event: &Event,
    kind: u32,
    author: &PublicKey,
    recipient: &PublicKey,
) -> Result<(), AgentFileError> {
    if event.kind.as_u16() as u32 != kind {
        return Err(AgentFileError::InvalidEnvelope(format!(
            "wrong kind: {}",
            event.kind.as_u16()
        )));
    }
    if &event.pubkey != author {
        return Err(AgentFileError::InvalidEnvelope(
            "pubkey != expected author".into(),
        ));
    }
    if single_hex_tag(event, "p")? != recipient.to_hex() {
        return Err(AgentFileError::InvalidEnvelope(
            "p tag != expected recipient".into(),
        ));
    }
    Ok(())
}

/// The value of the event's only `name` tag, which must be 64 lowercase hex
/// characters — anything else is non-canonical and would split heads.
fn single_hex_tag(event: &Event, name: &str) -> Result<String, AgentFileError> {
    let mut values = event
        .tags
        .iter()
        .filter(|t| t.kind().to_string() == name)
        .map(|t| t.content().unwrap_or_default().to_string());
    let value = values
        .next()
        .ok_or_else(|| AgentFileError::InvalidEnvelope(format!("missing {name} tag")))?;
    if values.next().is_some() {
        return Err(AgentFileError::InvalidEnvelope(format!(
            "multiple {name} tags"
        )));
    }
    if !is_sha256_hex(&value) {
        return Err(AgentFileError::InvalidEnvelope(format!(
            "{name} tag must be 64 lowercase hex chars"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECKEY_A: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    const SECKEY_O: &str = "0000000000000000000000000000000000000000000000000000000000000002";

    const D_NOTES: &str = "6a2ae802e89df6c3311cc145e4fd40280669b2853d13ca80b920ec7ac36a1160";
    const D_PLAN: &str = "f1a77048f250f512bdae879a96da177787f71a9b33e5218d255d99c8cf33a134";
    const D_BIG: &str = "3eba5168333f7a57b7d9967429fc681a45affe7e5a182e84d5c588c1f8da1d47";

    const BODY_NOTES: &str = r##"{"path":"notes.md","sha256":"38d997fefd1b7e6bb304b744dc708cc5e42ff41414e3e2878e3656c8f5ad02e4","size":19,"content":"hello, agent files\n"}"##;
    const BODY_PLAN: &str = r##"{"path":"PLANS/agent files.md","sha256":"c3964bb3b70a957ec9b233c7dd3653f6ba17701ab00facf88ae1393dc6155577","size":7,"content":"# Plan\n"}"##;
    const BODY_BIG: &str = r##"{"path":"logs/big.log","sha256":"66915c0872933db504e7578828dd85b7e74a4e0a061f9756793b89c4151bd4b5","size":70000}"##;
    const BODY_NOTES_REMOVED: &str = r##"{"path":"notes.md","removed":true}"##;
    const BODY_REQUEST: &str = r##"{"path":"PLANS/agent files.md","base_sha256":"c3964bb3b70a957ec9b233c7dd3653f6ba17701ab00facf88ae1393dc6155577","content":"# Plan\n\n- ship it\n"}"##;
    const BODY_RESULT: &str = r##"{"status":"applied","path":"PLANS/agent files.md","sha256":"e62b5d89e5ee431c4431bed125f015eaf4b952c81d55d5577487a5f1efc89786"}"##;

    /// `[content, id, sig]` of the pinned *Reference test vectors* events.
    const EVENT_NOTES: [&str; 3] = [
        "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABeWYcxyTrp5d68LBgA38mWthpZhLbxJYMfhJBIaLKj/BibYNSQQWLt6lkAXZKid+iG4Qbw5alru2O5QNHEsvgLDxoohNxwegWO+R8ZglzvKJvONpvb2DQgtDOCoAHlk4U79qce+Bc6EPESBbYYhyTsoZ3K4phyhQbPf4Wf17d6VrIjF90VGEnjdGQ8+kf9Q+LvFbLZK5Yy8TJmm+uSe6nruhmEJtEgN+E4HqYl3hm3wi0I7zDVq6XE5YP8+33SUgi3Fw=",
        "ffedcefb2ceca14ebd23fc5832be2eaece81364e5749b40551febb34ece68129",
        "851749264f0ee6cc356343942f6a8987687eeeb36718624459c8057ebd910676afa566280b06896b627e024b99a5ac65508c98becca7359873e2241ef9b46db6",
    ];
    const EVENT_PLAN: [&str; 3] = [
        "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACGzxBPvRUxTMAxOGGRKGhZiqgsEArQCRWg50Ke1gDJx+TGj9/Fewz3WSyeAfivT3k1gBothiBPJ5s4cOhoEO8II+l0uqLSfcuoPyWdKtqiGzypCM6179dYp0PvWim9j+wvCOLYun2TmeW8X9jYUNEs30aK7W0KxbM0JFd90vjbhlIh9YFQzQmV/gd5sN2BqBULXmmZgc5sMVzdb8DKkbG4CfkSf4eefdbfkQh4P8Adrk5gFTQ3MMFjBFsMXvjpLUQrAs=",
        "7057f1c1826c58d24cd61bd2ca084e6ebe3ac23e9d664deb286629e029d43da4",
        "c72a734e84af6b038e7c28dfefba7576b0bcb02f385bd38a99ce7c4795b867dc6d27e72d446aed33c2fcf4a211099fd55fc135ee3236f863180baa56a18ff751",
    ];
    const EVENT_BIG: [&str; 3] = [
        "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADufi8i0aj4uoLnp2rR/0icXfrLlipO1/EFjgIaMEZgjMtBxFfHRl2FE2vy5jJ/Z0ZfIqS0LWZbYrASsnxSWX0T0d1keXs2rhM8a/1YsmLEwPZUyetBJ6AUQpEoMjyKlwhJ8hHY02Z/WmOvJ+eAcSak81lFyUYqr/Q+r9Vu8cOmTJd+doLd9zrrbg1weUnGD1ZxHH403ggde05EdOMG+BgzCWm",
        "6ba315bfc0b986c56662e78e94de1a5df07fdee30e410e2b84e62b1608b3fba7",
        "5b73e8ffda0caeb76fac13150a2e617b2b4bb0b1c1a38dbee6619a5ee5852ed81ee1b0a3e6d80733e0d47f57625303988d842285880189c4656a0c4a81254947",
    ];
    const EVENT_NOTES_REMOVED: [&str; 3] = [
        "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEEZ1HAFvscs/AcKaYSSZ7c4DJyiq9MWbCVhh3F5cJYKzzNVHP0Co4mCrkLHwQYMHhOrSCVp2209axj77xAk5tDKg8mc1ZeRGDsVVtCEMHNYahgYEMzcE4bH/po+T3shhnl0c=",
        "1bec75664cb1939429d2f18b0a431dac23c5557668ac28665fa4ce7b2024c38a",
        "ff395138e65c57fa113c59c70e60f82a6ef47738a2de72fc9ab527c399f852836f7b74ce792b568ac7fec7986801d674c4881d87d21df4f87b4b413fc8bc0600",
    ];
    const EVENT_REQUEST: [&str; 3] = [
        "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFxUvaeNKl0ciXDyss+prpTNPK3paTrOuWRaJv0G9O6yQ57/2WtCobF8+8BMczu9OQI4Q9KwSBdWkY8Ip329LNeWAYp5u5pPq6pcxk/n3lIITYflSIQ+xSL+Q7Z3J4Nl1W4sFSQVcEqz7vBIYEE1qQDNS9vhdTUr+iNPkyvK4eTC4HtGywJUG1vzo4lQxPLf9go1gQAgGTZd97QXNgYSm9wmuVLpCkQm34rQJWejJvvI7CTY5h5m6BsjqGG39EahG9EzE=",
        "3fe347efff08d8f3da196d35a02d73207fe91cb463c8d122d9681af4552493f1",
        "06819ae9d529f977ebfb4e569efac6e232d13cbf197824f6302c373178b5d1e90228fa397217f58e10616dba1fd98d48e05ca673c2b7a400026df038a1fac0c9",
    ];
    const EVENT_RESULT: [&str; 3] = [
        "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAG2YpLVPS9j3qTMUV7xmqgMArZz1CjPEhhG0JsgG4XHf+FPtdlkCm4jFwS+VjJVE34cm3MoDwj2pcQhaj+htkGXJs4d7jd6KwsQ0BEMJzlNzuIPtLQz4n4ondlvElmfJ8A9ERMkc7h/IEgSUNSLF1J/9bVHdCtJeL7A08N2xOw5vsXZv/FO37yF0CVL7K6pCUlCPFHr6K/KbMaFHDFC8bu0Ajk",
        "5bc47d506d8eb863d57d5c00308f8e864d0f201ce5e881d58be43ab4ab3492e3",
        "3cc169f878ebda8ff972e3864475b14184dd14d8b6a62f22e88f2a84955391e4c8902761553a78e345245c4d6e2ce809d594f1cc4447a998f062424f3944ada9",
    ];

    fn spec_event(
        author: &Keys,
        kind: u32,
        created_at: u64,
        tags: serde_json::Value,
        [content, id, sig]: [&str; 3],
    ) -> Event {
        let event: Event = serde_json::from_value(serde_json::json!({
            "id": id,
            "pubkey": author.public_key().to_hex(),
            "created_at": created_at,
            "kind": kind,
            "tags": tags,
            "content": content,
            "sig": sig,
        }))
        .unwrap();
        event.verify().unwrap();
        event
    }

    fn agent() -> Keys {
        Keys::parse(SECKEY_A).unwrap()
    }

    fn owner() -> Keys {
        Keys::parse(SECKEY_O).unwrap()
    }

    fn file(path: &str, content: &str) -> Body {
        Body::for_contents(path.into(), content.as_bytes())
    }

    #[test]
    fn d_tags_match_spec() {
        let k_c = conversation_key(agent().secret_key(), &owner().public_key());
        assert_eq!(d_tag(&k_c, "notes.md"), D_NOTES);
        assert_eq!(d_tag(&k_c, "PLANS/agent files.md"), D_PLAN);
        assert_eq!(d_tag(&k_c, "logs/big.log"), D_BIG);
    }

    #[test]
    fn bodies_match_spec_byte_exact() {
        assert_eq!(
            file("notes.md", "hello, agent files\n").to_json_bytes(),
            BODY_NOTES.as_bytes()
        );
        assert_eq!(
            file("PLANS/agent files.md", "# Plan\n").to_json_bytes(),
            BODY_PLAN.as_bytes()
        );
        assert_eq!(
            Body::for_contents("logs/big.log".into(), "a".repeat(70_000).as_bytes())
                .to_json_bytes(),
            BODY_BIG.as_bytes()
        );
        assert_eq!(
            Body::Removed {
                path: "notes.md".into()
            }
            .to_json_bytes(),
            BODY_NOTES_REMOVED.as_bytes()
        );
        assert_eq!(
            EditRequest {
                path: "PLANS/agent files.md".into(),
                base_sha256: sha256_hex(b"# Plan\n"),
                content: "# Plan\n\n- ship it\n".into(),
            }
            .to_json_bytes(),
            BODY_REQUEST.as_bytes()
        );
        assert_eq!(
            EditResult {
                status: EditStatus::Applied,
                path: "PLANS/agent files.md".into(),
                sha256: Some(sha256_hex(b"# Plan\n\n- ship it\n")),
                reason: None,
            }
            .to_json_bytes(),
            BODY_RESULT.as_bytes()
        );
    }

    #[test]
    fn spec_events_validate_and_decrypt() {
        let (agent, owner) = (agent(), owner());
        let (a, o) = (agent.public_key(), owner.public_key());
        let p_owner = serde_json::json!([["p", o.to_hex()]]);
        let record = |d: &str| serde_json::json!([["d", d], ["p", o.to_hex()]]);
        let records = [
            (1_700_000_000, D_NOTES, EVENT_NOTES, BODY_NOTES),
            (1_700_000_001, D_PLAN, EVENT_PLAN, BODY_PLAN),
            (1_700_000_002, D_BIG, EVENT_BIG, BODY_BIG),
            (
                1_700_000_003,
                D_NOTES,
                EVENT_NOTES_REMOVED,
                BODY_NOTES_REMOVED,
            ),
        ];
        let mut notes = Vec::new();
        for (created_at, d, event, body) in records {
            let event = spec_event(&agent, KIND_AGENT_FILE, created_at, record(d), event);
            let decoded = validate_and_decrypt(&event, &a, &o, owner.secret_key(), &a).unwrap();
            assert_eq!(decoded, Body::from_json_bytes(body.as_bytes()).unwrap());
            if d == D_NOTES {
                notes.push(event);
            }
        }
        let head = select_head(notes).unwrap();
        assert_eq!(
            head.id.to_hex(),
            EVENT_NOTES_REMOVED[1],
            "tombstone is the head"
        );

        let request = spec_event(
            &owner,
            KIND_AGENT_FILE_EDIT_REQUEST,
            1_700_000_004,
            serde_json::json!([["p", a.to_hex()]]),
            EVENT_REQUEST,
        );
        assert_eq!(
            decrypt_edit_request(&request, &o, &a, agent.secret_key(), &o)
                .unwrap()
                .to_json_bytes(),
            BODY_REQUEST.as_bytes()
        );

        let mut result_tags = p_owner;
        result_tags
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!(["e", EVENT_REQUEST[1]]));
        let result = spec_event(
            &agent,
            KIND_AGENT_FILE_EDIT_RESULT,
            1_700_000_005,
            result_tags,
            EVENT_RESULT,
        );
        let (answered, decoded) =
            decrypt_edit_result(&result, &a, &o, owner.secret_key(), &a).unwrap();
        assert_eq!(answered, request.id);
        assert_eq!(decoded.to_json_bytes(), BODY_RESULT.as_bytes());
    }

    #[test]
    fn d_tag_is_domain_separated_from_engrams() {
        let k_c = conversation_key(agent().secret_key(), &owner().public_key());
        assert_ne!(d_tag(&k_c, "core"), crate::engram::d_tag(&k_c, "core"));
        assert_eq!(
            d_tag(&k_c, "notes.md"),
            d_tag(
                &conversation_key(owner().secret_key(), &agent().public_key()),
                "notes.md"
            ),
            "owner and agent derive the same d tag"
        );
    }

    #[test]
    fn validate_path_accepts_grammar() {
        for ok in [
            "a",
            "notes.md",
            "PLANS/agent files.md",
            ".config/settings.json",
            "a/b/c/d.txt",
            "ünïcödé/日本.md",
            "...",
        ] {
            assert!(validate_path(ok).is_ok(), "{ok:?} should be valid");
        }
        assert!(validate_path(&"a".repeat(PATH_MAX_LEN)).is_ok());
    }

    #[test]
    fn validate_path_rejects_garbage() {
        for bad in [
            "",
            "/etc/passwd",
            "a/",
            "a//b",
            "./a",
            "a/./b",
            "../a",
            "a/../b",
            "..",
            "a\\b",
            "a\nb",
            "a\0b",
            "a\u{7f}b",
        ] {
            assert!(validate_path(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(validate_path(&"a".repeat(PATH_MAX_LEN + 1)).is_err());
    }

    #[test]
    fn body_round_trips_byte_exact() {
        let body = file("PLANS/x.md", "# Plan\n\"quoted\"\n");
        let json = br##"{"path":"PLANS/x.md","sha256":"603798946327a45d30bd5c6e812293277694a9497e1b2462a52cb2ef7b730ad1","size":16,"content":"# Plan\n\"quoted\"\n"}"##;
        assert_eq!(body.to_json_bytes(), json.to_vec());
        assert_eq!(Body::from_json_bytes(json).unwrap(), body);

        let removed = Body::Removed {
            path: "PLANS/x.md".into(),
        };
        assert_eq!(
            removed.to_json_bytes(),
            br#"{"path":"PLANS/x.md","removed":true}"#.to_vec()
        );
        assert_eq!(
            Body::from_json_bytes(&removed.to_json_bytes()).unwrap(),
            removed
        );
    }

    #[test]
    fn for_contents_lists_binary_and_oversized_files_without_content() {
        let binary = Body::for_contents("img.png".into(), &[0x89, b'P', b'N', b'G', 0xff]);
        assert!(matches!(
            binary,
            Body::File {
                content: None,
                size: 5,
                ..
            }
        ));

        let fits = "a".repeat(NIP44_PLAINTEXT_MAX - 200);
        assert!(matches!(
            Body::for_contents("big.txt".into(), fits.as_bytes()),
            Body::File {
                content: Some(_),
                ..
            }
        ));

        let huge = "a".repeat(NIP44_PLAINTEXT_MAX);
        let body = Body::for_contents("huge.txt".into(), huge.as_bytes());
        assert!(matches!(
            body,
            Body::File {
                content: None,
                size,
                ..
            } if size == NIP44_PLAINTEXT_MAX as u64
        ));
        assert!(build_file_event(&agent(), &owner().public_key(), &body, 1).is_ok());
    }

    #[test]
    fn body_parse_rejects_content_that_does_not_match_its_hash() {
        let tampered = format!(
            r#"{{"path":"a.md","sha256":"{}","size":5,"content":"world"}}"#,
            sha256_hex(b"hello")
        );
        let err = Body::from_json_bytes(tampered.as_bytes()).unwrap_err();
        assert!(matches!(err, AgentFileError::InvalidBody(_)), "got {err}");

        let wrong_size = format!(
            r#"{{"path":"a.md","sha256":"{}","size":6,"content":"hello"}}"#,
            sha256_hex(b"hello")
        );
        assert!(Body::from_json_bytes(wrong_size.as_bytes()).is_err());
    }

    #[test]
    fn body_parse_rejects_bad_shapes() {
        let sha = sha256_hex(b"");
        for bad in [
            r#"{"path":"a","removed":false}"#.to_string(),
            r#"{"path":"a","removed":"yes"}"#.to_string(),
            r#"{"path":"a","size":0}"#.to_string(),
            format!(r#"{{"path":"a","sha256":"{sha}"}}"#),
            format!(r#"{{"path":"a","sha256":"{sha}","size":-1}}"#),
            format!(
                r#"{{"path":"a","sha256":"{}","size":0}}"#,
                sha.to_uppercase()
            ),
            format!(r#"{{"path":"../a","sha256":"{sha}","size":0}}"#),
            format!(r#"{{"path":"a","path":"b","sha256":"{sha}","size":0}}"#),
            format!(r#"{{"path":"a","sha256":"{sha}","size":0,"content":null}}"#),
            r#"["a"]"#.to_string(),
        ] {
            assert!(
                Body::from_json_bytes(bad.as_bytes()).is_err(),
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn body_parse_ignores_unknown_fields() {
        let body = format!(
            r#"{{"path":"a","sha256":"{}","size":0,"mode":"0644","future":{{"x":1}}}}"#,
            sha256_hex(b"")
        );
        assert!(Body::from_json_bytes(body.as_bytes()).is_ok());
    }

    #[test]
    fn file_event_round_trips_for_agent_and_owner() {
        let (agent, owner) = (agent(), owner());
        let body = file("notes/today.md", "hello");
        let event = build_file_event(&agent, &owner.public_key(), &body, 1_700_000_000).unwrap();
        assert_eq!(event.kind.as_u16() as u32, KIND_AGENT_FILE);
        assert_eq!(event.created_at.as_secs(), 1_700_000_000);
        let as_owner = validate_and_decrypt(
            &event,
            &agent.public_key(),
            &owner.public_key(),
            owner.secret_key(),
            &agent.public_key(),
        )
        .unwrap();
        let as_agent = validate_and_decrypt(
            &event,
            &agent.public_key(),
            &owner.public_key(),
            agent.secret_key(),
            &owner.public_key(),
        )
        .unwrap();
        assert_eq!(as_owner, body);
        assert_eq!(as_agent, body);
    }

    #[test]
    fn file_event_rejects_path_that_does_not_rederive_to_d() {
        let (agent, owner) = (agent(), owner());
        let real = build_file_event(&agent, &owner.public_key(), &file("a.md", "x"), 1).unwrap();
        let other = build_file_event(&agent, &owner.public_key(), &file("b.md", "x"), 1).unwrap();
        let swapped = EventBuilder::new(real.kind, other.content.clone())
            .tags(real.tags.clone())
            .sign_with_keys(&agent)
            .unwrap();
        let err = validate_and_decrypt(
            &swapped,
            &agent.public_key(),
            &owner.public_key(),
            owner.secret_key(),
            &agent.public_key(),
        )
        .unwrap_err();
        assert!(
            matches!(err, AgentFileError::InvalidEnvelope(_)),
            "got {err}"
        );
    }

    #[test]
    fn file_event_rejects_records_signed_by_the_owner() {
        let (agent, owner) = (agent(), owner());
        let forged = build_file_event(&owner, &agent.public_key(), &file("a.md", "x"), 1).unwrap();
        let err = validate_and_decrypt(
            &forged,
            &agent.public_key(),
            &owner.public_key(),
            owner.secret_key(),
            &agent.public_key(),
        )
        .unwrap_err();
        assert!(
            matches!(err, AgentFileError::InvalidEnvelope(_)),
            "got {err}"
        );
    }

    #[test]
    fn edit_request_round_trips_and_binds_direction() {
        let (agent, owner) = (agent(), owner());
        let request = EditRequest {
            path: "PLANS/x.md".into(),
            base_sha256: sha256_hex(b"old"),
            content: "new".into(),
        };
        let event = build_edit_request(&owner, &agent.public_key(), &request).unwrap();
        assert_eq!(event.kind.as_u16() as u32, KIND_AGENT_FILE_EDIT_REQUEST);
        let decoded = decrypt_edit_request(
            &event,
            &owner.public_key(),
            &agent.public_key(),
            agent.secret_key(),
            &owner.public_key(),
        )
        .unwrap();
        assert_eq!(decoded, request);

        let from_agent = build_edit_request(&agent, &owner.public_key(), &request).unwrap();
        assert!(decrypt_edit_request(
            &from_agent,
            &owner.public_key(),
            &agent.public_key(),
            agent.secret_key(),
            &owner.public_key(),
        )
        .is_err());
    }

    #[test]
    fn edit_result_round_trips_with_request_reference() {
        let (agent, owner) = (agent(), owner());
        let request = build_edit_request(
            &owner,
            &agent.public_key(),
            &EditRequest {
                path: "a.md".into(),
                base_sha256: sha256_hex(b"old"),
                content: "new".into(),
            },
        )
        .unwrap();
        for result in [
            EditResult {
                status: EditStatus::Applied,
                path: "a.md".into(),
                sha256: Some(sha256_hex(b"new")),
                reason: None,
            },
            EditResult {
                status: EditStatus::Conflict,
                path: "a.md".into(),
                sha256: Some(sha256_hex(b"newer")),
                reason: None,
            },
            EditResult {
                status: EditStatus::Declined,
                path: "a.md".into(),
                sha256: None,
                reason: Some("read-only".into()),
            },
        ] {
            let event =
                build_edit_result(&agent, &owner.public_key(), &request.id, &result).unwrap();
            let (answered, decoded) = decrypt_edit_result(
                &event,
                &agent.public_key(),
                &owner.public_key(),
                owner.secret_key(),
                &agent.public_key(),
            )
            .unwrap();
            assert_eq!(answered, request.id);
            assert_eq!(decoded, result);
        }
    }

    #[test]
    fn edit_result_requires_sha256_when_applied() {
        let body = br#"{"status":"applied","path":"a.md"}"#;
        assert!(EditResult::from_json_bytes(body).is_err());
        let body = br#"{"status":"merged","path":"a.md"}"#;
        assert!(EditResult::from_json_bytes(body).is_err());
    }

    #[test]
    fn edit_result_without_e_tag_is_rejected() {
        let (agent, owner) = (agent(), owner());
        let result = EditResult {
            status: EditStatus::Declined,
            path: "a.md".into(),
            sha256: None,
            reason: None,
        };
        let untagged = seal(
            &agent,
            &owner.public_key(),
            KIND_AGENT_FILE_EDIT_RESULT,
            &result.to_json_bytes(),
            &[["p".to_string(), owner.public_key().to_hex()]],
            None,
        )
        .unwrap();
        let err = decrypt_edit_result(
            &untagged,
            &agent.public_key(),
            &owner.public_key(),
            owner.secret_key(),
            &agent.public_key(),
        )
        .unwrap_err();
        assert!(
            matches!(err, AgentFileError::InvalidEnvelope(_)),
            "got {err}"
        );
    }
}
