//! Connect an agent that runs elsewhere and already holds its own key.
//!
//! The owner attests the agent's public key (NIP-OA) and publishes its
//! owner-signed kind:30177 policy; the agent's secret key never reaches this
//! device. The attestation becomes owner provenance only once the agent
//! presents it, which needs the agent's key: see `docs/owned-agent-discovery.md`.

use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
use nostr::{Event, PublicKey};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use super::agents::tombstone_managed_agent_at;
use crate::{
    app_state::AppState,
    managed_agents::{
        agent_events::{
            agent_event_builder, managed_agent_content_from_event, ManagedAgentEventContent,
        },
        load_managed_agents,
        persona_events::flush_pending_events_at,
        reconcile::retain_agent_event,
        retention::{active_retention_scope, open_retention_db, RetentionScope},
        validate_managed_agent_definition_text, validate_respond_to_allowlist, RespondTo,
    },
    nostr_convert::verified_agent_owners_from_profiles,
    relay::{query_relay_at, relay_http_base_url},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectManagedAgentRequest {
    pub agent_pubkey: String,
    pub name: String,
    #[serde(default)]
    pub respond_to: RespondTo,
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectManagedAgentResponse {
    pub agent_pubkey: String,
    pub auth_tag: String,
    pub relay_url: String,
    /// The owner policy on record for the agent after this call.
    pub policy: ManagedAgentEventContent,
    /// True when an existing policy was kept instead of the requested one.
    pub policy_kept: bool,
    pub policy_sync_error: Option<String>,
}

fn parse_agent_pubkey(input: &str) -> Result<PublicKey, String> {
    PublicKey::parse(input.trim())
        .map_err(|_| "Enter the agent's public key as an npub or 64-character hex key.".to_string())
}

/// The policy a connected agent publishes. The runtime is the operator's, so
/// the record carries no definition fields.
fn connected_agent_content(
    name: String,
    respond_to: RespondTo,
    respond_to_allowlist: Vec<String>,
) -> ManagedAgentEventContent {
    ManagedAgentEventContent {
        name,
        persona_id: None,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        parallelism: 1,
        respond_to,
        respond_to_allowlist,
    }
}

/// The owner's policy already on record for `agent` among the relay's
/// `events`, refusing an agent whose own profile attests a different owner.
fn existing_owner_policy(
    events: &[Event],
    owner: &PublicKey,
    agent: &PublicKey,
) -> Result<Option<ManagedAgentEventContent>, String> {
    let agent_hex = agent.to_hex();
    if verified_agent_owners_from_profiles(events)
        .get(&agent_hex)
        .is_some_and(|attested| *attested != owner.to_hex())
    {
        return Err("This agent is already attested by another owner.".to_string());
    }
    events
        .iter()
        .filter(|event| {
            event.kind == nostr::Kind::Custom(KIND_MANAGED_AGENT as u16)
                && event.pubkey == *owner
                && event.tags.identifier() == Some(agent_hex.as_str())
        })
        .max_by_key(|event| event.created_at)
        .map(managed_agent_content_from_event)
        .transpose()
}

fn ensure_not_managed_here(app: &AppHandle, state: &AppState, agent: &str) -> Result<(), String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    if load_managed_agents(app)?
        .iter()
        .any(|record| record.pubkey.eq_ignore_ascii_case(agent))
    {
        return Err("This agent is managed on this device.".to_string());
    }
    Ok(())
}

/// Attest an existing agent key and publish its owner policy.
///
/// Returns the NIP-OA auth tag for the operator to install as `BUZZ_AUTH_TAG`.
/// An agent that already has a policy from this owner keeps it: connecting
/// again only issues a new tag, so it never overwrites another device's record.
#[tauri::command]
pub async fn connect_managed_agent(
    input: ConnectManagedAgentRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ConnectManagedAgentResponse, String> {
    let scope = active_retention_scope(&app, &state)?;
    ensure_not_managed_here(
        &app,
        &state,
        &parse_agent_pubkey(&input.agent_pubkey)?.to_hex(),
    )?;
    connect_agent_at(&state, &scope, input).await
}

/// Scope-free core of [`connect_managed_agent`].
async fn connect_agent_at(
    state: &AppState,
    scope: &RetentionScope,
    input: ConnectManagedAgentRequest,
) -> Result<ConnectManagedAgentResponse, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("Name the agent.".to_string());
    }
    validate_managed_agent_definition_text(&name, None, None)?;
    let agent = parse_agent_pubkey(&input.agent_pubkey)?;
    let agent_pubkey = agent.to_hex();
    let respond_to_allowlist = validate_respond_to_allowlist(&input.respond_to_allowlist)?;
    if input.respond_to == RespondTo::Allowlist && respond_to_allowlist.is_empty() {
        return Err(
            "respond-to mode 'allowlist' requires at least one pubkey in the allowlist".to_string(),
        );
    }
    let owner = scope.owner_keys.public_key();
    if owner == agent {
        return Err("That is your own key. Connect the agent's key instead.".to_string());
    }

    let events = query_relay_at(
        state,
        &relay_http_base_url(&scope.relay_url),
        &[
            serde_json::json!({
                "kinds": [0],
                "authors": [&agent_pubkey],
                "limit": 1,
            }),
            serde_json::json!({
                "kinds": [KIND_MANAGED_AGENT],
                "authors": [owner.to_hex()],
                "#d": [&agent_pubkey],
                "limit": 1,
            }),
        ],
    )
    .await?;
    let existing_policy = existing_owner_policy(&events, &owner, &agent)?;
    let auth_tag = buzz_sdk_pkg::nip_oa::compute_auth_tag(&scope.owner_keys, &agent, "")
        .map_err(|e| format!("failed to compute NIP-OA auth tag: {e}"))?;

    let policy_kept = existing_policy.is_some();
    let policy = match existing_policy {
        Some(policy) => policy,
        None => {
            let policy = connected_agent_content(name, input.respond_to, respond_to_allowlist);
            let conn = open_retention_db(&scope.db_path)?;
            retain_agent_event(
                &conn,
                &scope.owner_keys,
                &agent_pubkey,
                agent_event_builder(&agent_pubkey, &policy)?,
            )?;
            policy
        }
    };
    let policy_sync_error =
        flush_pending_events_at(&scope.db_path, state, &scope.relay_url, &scope.owner_keys)
            .await
            .err();

    Ok(ConnectManagedAgentResponse {
        agent_pubkey,
        auth_tag,
        relay_url: scope.relay_url.clone(),
        policy,
        policy_kept,
        policy_sync_error,
    })
}

/// Withdraw the owner policy of an agent this device does not manage and
/// archive its identity.
///
/// The agent's NIP-OA tag stays valid: an owner cannot revoke a signature it
/// already handed out. Returns the relay sync error, if any; the tombstone is
/// retained and retried by the flush loop.
#[tauri::command]
pub async fn disconnect_managed_agent(
    agent_pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let agent_pubkey = parse_agent_pubkey(&agent_pubkey)?.to_hex();
    let scope = active_retention_scope(&app, &state)?;
    ensure_not_managed_here(&app, &state, &agent_pubkey)?;
    tombstone_managed_agent_at(&scope.db_path, &scope.owner_keys, &agent_pubkey)?;
    Ok(
        flush_pending_events_at(&scope.db_path, &state, &scope.relay_url, &scope.owner_keys)
            .await
            .err(),
    )
}

#[cfg(test)]
#[path = "agent_connect_tests.rs"]
mod tests;
