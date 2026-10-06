//! Owner-side NIP-AF agent files for the desktop: the files an agent shares
//! with its owner, and the owner's edit requests to them.
//!
//! Mirrors `engrams.rs`: one Tauri call per panel open returns the decrypted
//! listing together with every recent edit request and its answer, decrypted
//! with the owner's own key. Proposing an edit signs a `kind:4180` as the
//! owner; the agent applies it (or not) and answers with a `kind:4181`.

use std::collections::HashMap;
use std::time::SystemTime;

use nostr::{Event, Keys, PublicKey};
use serde::Serialize;
use tauri::{AppHandle, State};

use buzz_core_pkg::agent_files::{
    build_edit_request, decrypt_edit_request, decrypt_edit_result, select_head,
    validate_and_decrypt, Body, EditRequest, EditResult, EditStatus,
};
use buzz_core_pkg::kind::{
    KIND_AGENT_FILE, KIND_AGENT_FILE_EDIT_REQUEST, KIND_AGENT_FILE_EDIT_RESULT,
};

use crate::app_state::AppState;
use crate::commands::engrams::authorize_agent_owner;
use crate::relay::{
    query_relay, relay_api_base_url_with_override, submit_signed_event_at_with_keys,
};

/// Cap on file records per (agent, owner) pair, as for engrams.
const AGENT_FILE_FETCH_LIMIT: u32 = 5000;
/// Cap on edit requests (and, separately, results) fetched per panel open.
const EDIT_FETCH_LIMIT: u32 = 200;

/// One shared file. `content` is `None` when the agent lists the file
/// without inlining it (not text, or too large).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub content: Option<String>,
    pub event_id: String,
    pub created_at: u64,
}

/// Where an edit request stands: `Pending` until the agent answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentFileEditStatus {
    Pending,
    Applied,
    Conflict,
    Declined,
}

impl From<EditStatus> for AgentFileEditStatus {
    fn from(status: EditStatus) -> Self {
        match status {
            EditStatus::Applied => Self::Applied,
            EditStatus::Conflict => Self::Conflict,
            EditStatus::Declined => Self::Declined,
        }
    }
}

/// One of the owner's edit requests and the agent's answer, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFileEdit {
    pub request_id: String,
    pub path: String,
    pub base_sha256: String,
    pub content: String,
    pub created_at: u64,
    pub status: AgentFileEditStatus,
    /// The file's hash after the answer (new on `applied`, current on `conflict`).
    pub sha256: Option<String>,
    pub reason: Option<String>,
    pub answered_at: Option<u64>,
}

/// Single-payload response for one panel open.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFilesListing {
    /// Every shared file, tombstones dropped. Sorted by path.
    pub files: Vec<AgentFileEntry>,
    /// The owner's edit requests, newest first.
    pub edits: Vec<AgentFileEdit>,
    /// True if the relay returned `>= AGENT_FILE_FETCH_LIMIT` file records.
    pub truncated: bool,
    pub fetched_at: u64,
}

/// Head per path, tombstones dropped, sorted by path (NIP-AF *Listing*).
/// Events that fail validation are skipped, so one bad record can't take
/// the whole listing down.
fn list_files(
    events: &[Event],
    agent: &PublicKey,
    owner: &PublicKey,
    owner_keys: &Keys,
) -> Vec<AgentFileEntry> {
    let mut groups: HashMap<String, Vec<(Event, Body)>> = HashMap::new();
    for event in events {
        if let Ok(body) = validate_and_decrypt(event, agent, owner, owner_keys.secret_key(), agent)
        {
            groups
                .entry(body.path().to_string())
                .or_default()
                .push((event.clone(), body));
        }
    }
    let mut files: Vec<AgentFileEntry> = groups
        .into_values()
        .filter_map(|members| {
            let head = select_head(members.iter().map(|(e, _)| e.clone()))?;
            let (_, body) = members.into_iter().find(|(e, _)| e.id == head.id)?;
            match body {
                Body::File {
                    path,
                    sha256,
                    size,
                    content,
                } => Some(AgentFileEntry {
                    path,
                    sha256,
                    size,
                    content,
                    event_id: head.id.to_hex(),
                    created_at: head.created_at.as_secs(),
                }),
                Body::Removed { .. } => None,
            }
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// The owner's requests, newest first, each with the earliest valid answer
/// that e-tags it.
fn list_edits(
    requests: &[Event],
    results: &[Event],
    agent: &PublicKey,
    owner: &PublicKey,
    owner_keys: &Keys,
) -> Vec<AgentFileEdit> {
    let mut answers: HashMap<nostr::EventId, (&Event, EditResult)> = HashMap::new();
    for event in results {
        let Ok((request_id, result)) =
            decrypt_edit_result(event, agent, owner, owner_keys.secret_key(), agent)
        else {
            continue;
        };
        let earlier = answers
            .get(&request_id)
            .is_some_and(|(prior, _)| (prior.created_at, prior.id) <= (event.created_at, event.id));
        if !earlier {
            answers.insert(request_id, (event, result));
        }
    }
    let mut edits: Vec<AgentFileEdit> = requests
        .iter()
        .filter_map(|event| {
            let request =
                decrypt_edit_request(event, owner, agent, owner_keys.secret_key(), agent).ok()?;
            let answer = answers.get(&event.id);
            Some(AgentFileEdit {
                request_id: event.id.to_hex(),
                path: request.path,
                base_sha256: request.base_sha256,
                content: request.content,
                created_at: event.created_at.as_secs(),
                status: answer.map_or(AgentFileEditStatus::Pending, |(_, r)| r.status.into()),
                sha256: answer.and_then(|(_, r)| r.sha256.clone()),
                reason: answer.and_then(|(_, r)| r.reason.clone()),
                answered_at: answer.map(|(e, _)| e.created_at.as_secs()),
            })
        })
        .collect();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.created_at));
    edits
}

/// `get_agent_files` — owner-gated listing of an agent's shared files and
/// the owner's edit requests to them.
#[tauri::command]
pub async fn get_agent_files(
    agent_pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentFilesListing, String> {
    let agent = authorize_agent_owner(&agent_pubkey, &app, &state).await?;
    let owner_keys = state.keys.lock().map_err(|e| e.to_string())?.clone();
    let owner = owner_keys.public_key();

    let filters = [
        serde_json::json!({
            "kinds": [KIND_AGENT_FILE],
            "authors": [agent.to_hex()],
            "#p": [owner.to_hex()],
            "limit": AGENT_FILE_FETCH_LIMIT,
        }),
        serde_json::json!({
            "kinds": [KIND_AGENT_FILE_EDIT_REQUEST],
            "authors": [owner.to_hex()],
            "#p": [agent.to_hex()],
            "limit": EDIT_FETCH_LIMIT,
        }),
        serde_json::json!({
            "kinds": [KIND_AGENT_FILE_EDIT_RESULT],
            "authors": [agent.to_hex()],
            "#p": [owner.to_hex()],
            "limit": EDIT_FETCH_LIMIT,
        }),
    ];
    let events: Vec<Event> = query_relay(&state, &filters)
        .await?
        .into_iter()
        .filter(|e| e.verify().is_ok())
        .collect();
    let of_kind = |kind: u32| -> Vec<Event> {
        events
            .iter()
            .filter(|e| e.kind.as_u16() as u32 == kind)
            .cloned()
            .collect()
    };
    let records = of_kind(KIND_AGENT_FILE);
    let truncated = records.len() as u32 >= AGENT_FILE_FETCH_LIMIT;

    Ok(AgentFilesListing {
        files: list_files(&records, &agent, &owner, &owner_keys),
        edits: list_edits(
            &of_kind(KIND_AGENT_FILE_EDIT_REQUEST),
            &of_kind(KIND_AGENT_FILE_EDIT_RESULT),
            &agent,
            &owner,
            &owner_keys,
        ),
        truncated,
        fetched_at: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    })
}

/// `propose_agent_file_edit` — send the agent a full replacement for one of
/// its shared files, based on the version the owner edited. Returns the
/// request's event id; the answer arrives as a `kind:4181` the next listing
/// picks up.
#[tauri::command]
pub async fn propose_agent_file_edit(
    agent_pubkey: String,
    path: String,
    base_sha256: String,
    content: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let agent = authorize_agent_owner(&agent_pubkey, &app, &state).await?;
    let owner_keys = state.signing_keys()?;
    let request = EditRequest {
        path,
        base_sha256,
        content,
    };
    let event = build_edit_request(&owner_keys, &agent, &request).map_err(|e| e.to_string())?;
    let api_base_url = relay_api_base_url_with_override(&state);
    submit_signed_event_at_with_keys(&event, &state, &api_base_url, &owner_keys).await?;
    Ok(event.id.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core_pkg::agent_files::{build_edit_result, build_file_event, sha256_hex};

    fn file(path: &str, content: &str) -> Body {
        Body::for_contents(path.into(), content.as_bytes())
    }

    fn request(owner: &Keys, agent: &Keys, path: &str, base: &str, content: &str) -> Event {
        build_edit_request(
            owner,
            &agent.public_key(),
            &EditRequest {
                path: path.into(),
                base_sha256: sha256_hex(base.as_bytes()),
                content: content.into(),
            },
        )
        .unwrap()
    }

    fn answer(agent: &Keys, owner: &Keys, request: &Event, status: EditStatus) -> Event {
        build_edit_result(
            agent,
            &owner.public_key(),
            &request.id,
            &EditResult {
                status,
                path: "a.md".into(),
                sha256: Some(sha256_hex(b"new")),
                reason: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn list_files_takes_heads_and_drops_tombstones() {
        let (agent, owner) = (Keys::generate(), Keys::generate());
        let o = owner.public_key();
        let events = [
            build_file_event(&agent, &o, &file("b.md", "old"), 1).unwrap(),
            build_file_event(&agent, &o, &file("b.md", "new"), 2).unwrap(),
            build_file_event(&agent, &o, &file("a.md", "a"), 1).unwrap(),
            build_file_event(&agent, &o, &file("gone.md", "x"), 1).unwrap(),
            build_file_event(
                &agent,
                &o,
                &Body::Removed {
                    path: "gone.md".into(),
                },
                2,
            )
            .unwrap(),
            build_file_event(&owner, &agent.public_key(), &file("forged.md", "x"), 3).unwrap(),
        ];
        let files = list_files(&events, &agent.public_key(), &o, &owner);
        let summary: Vec<_> = files
            .iter()
            .map(|f| (f.path.as_str(), f.content.as_deref()))
            .collect();
        assert_eq!(summary, [("a.md", Some("a")), ("b.md", Some("new"))]);
    }

    #[test]
    fn list_edits_pairs_requests_with_their_earliest_answer() {
        let (agent, owner) = (Keys::generate(), Keys::generate());
        let applied = request(&owner, &agent, "a.md", "old", "new");
        let pending = request(&owner, &agent, "b.md", "old", "new");
        let results = [answer(&agent, &owner, &applied, EditStatus::Applied)];
        let edits = list_edits(
            &[applied.clone(), pending.clone()],
            &results,
            &agent.public_key(),
            &owner.public_key(),
            &owner,
        );
        let by_id: HashMap<_, _> = edits.iter().map(|e| (e.request_id.clone(), e)).collect();
        let a = by_id[&applied.id.to_hex()];
        assert_eq!(a.status, AgentFileEditStatus::Applied);
        assert_eq!(a.sha256, Some(sha256_hex(b"new")));
        assert!(a.answered_at.is_some());
        let b = by_id[&pending.id.to_hex()];
        assert_eq!(b.status, AgentFileEditStatus::Pending);
        assert_eq!(b.content, "new");
    }

    #[test]
    fn list_edits_ignores_answers_from_anyone_but_the_agent() {
        let (agent, owner, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
        let pending = request(&owner, &agent, "a.md", "old", "new");
        let forged = answer(&stranger, &owner, &pending, EditStatus::Applied);
        let edits = list_edits(
            &[pending],
            &[forged],
            &agent.public_key(),
            &owner.public_key(),
            &owner,
        );
        assert_eq!(edits[0].status, AgentFileEditStatus::Pending);
    }
}
