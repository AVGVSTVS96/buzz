use nostr::{Event, EventBuilder, Keys, Kind, Tag, ToBech32};

use super::*;
use crate::nostr_convert::relay_agents_from_directory_events;

fn connected_policy(owner: &Keys, agent: &PublicKey, respond_to: RespondTo) -> Event {
    agent_event_builder(
        &agent.to_hex(),
        &connected_agent_content("Hex".to_string(), respond_to, Vec::new()),
    )
    .unwrap()
    .sign_with_keys(owner)
    .unwrap()
}

fn profile_presenting_tag(owner: &Keys, agent: &Keys) -> Event {
    let auth_tag = buzz_sdk_pkg::nip_oa::compute_auth_tag(owner, &agent.public_key(), "").unwrap();
    let auth_tag: Vec<String> = serde_json::from_str(&auth_tag).unwrap();
    EventBuilder::new(Kind::Metadata, r#"{"name":"Hex"}"#)
        .tags([Tag::parse(auth_tag).unwrap()])
        .sign_with_keys(agent)
        .unwrap()
}

#[test]
fn connected_policy_is_discovered_once_the_agent_presents_its_tag() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let policy = connected_policy(&owner, &agent.public_key(), RespondTo::Anyone);

    let agents = relay_agents_from_directory_events(
        &[],
        std::slice::from_ref(&policy),
        &[profile_presenting_tag(&owner, &agent)],
    );

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].pubkey, agent.public_key().to_hex());
    assert_eq!(agents[0].owner_pubkey, Some(owner.public_key().to_hex()));
    assert_eq!(agents[0].name, "Hex");
    assert_eq!(agents[0].respond_to, Some(RespondTo::Anyone));
}

#[test]
fn connected_policy_alone_does_not_claim_the_key() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let policy = connected_policy(&owner, &agent.public_key(), RespondTo::Anyone);

    assert!(relay_agents_from_directory_events(&[], std::slice::from_ref(&policy), &[]).is_empty());
    let attested_elsewhere = relay_agents_from_directory_events(
        &[],
        &[policy],
        &[profile_presenting_tag(&Keys::generate(), &agent)],
    );
    assert!(attested_elsewhere
        .iter()
        .all(|found| found.owner_pubkey != Some(owner.public_key().to_hex())));
}

#[test]
fn agent_pubkey_accepts_npub_or_hex_and_never_a_secret_key() {
    let keys = Keys::generate();
    let agent = keys.public_key();
    let npub = agent.to_bech32().unwrap();

    assert_eq!(parse_agent_pubkey(&format!(" {npub} ")).unwrap(), agent);
    assert_eq!(parse_agent_pubkey(&agent.to_hex()).unwrap(), agent);
    assert!(parse_agent_pubkey(&npub[..npub.len() - 1]).is_err());
    assert!(parse_agent_pubkey(&keys.secret_key().to_bech32().unwrap()).is_err());
}

#[test]
fn an_existing_owner_policy_is_kept() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let policy = connected_policy(&owner, &agent.public_key(), RespondTo::Anyone);
    let foreign_policy =
        connected_policy(&Keys::generate(), &agent.public_key(), RespondTo::OwnerOnly);

    let kept = existing_owner_policy(
        &[
            foreign_policy,
            policy,
            profile_presenting_tag(&owner, &agent),
        ],
        &owner.public_key(),
        &agent.public_key(),
    )
    .unwrap();

    assert_eq!(
        kept.map(|policy| policy.respond_to),
        Some(RespondTo::Anyone)
    );
    assert_eq!(
        existing_owner_policy(&[], &owner.public_key(), &agent.public_key()).unwrap(),
        None
    );
}

/// A loopback relay that answers every query with its stored events and
/// stores every accepted publish.
async fn loopback_relay() -> (String, std::sync::Arc<std::sync::Mutex<Vec<Event>>>) {
    use axum::{routing::post, Json, Router};

    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Event>::new()));
    let (query_events, publish_events) = (events.clone(), events.clone());
    let router = Router::new()
        .route(
            "/query",
            post(move || {
                let events = query_events.clone();
                async move { Json(events.lock().unwrap().clone()) }
            }),
        )
        .route(
            "/events",
            post(move |Json(event): Json<Event>| {
                let events = publish_events.clone();
                async move {
                    let event_id = event.id.to_hex();
                    events.lock().unwrap().push(event);
                    Json(serde_json::json!({"event_id": event_id, "accepted": true, "message": ""}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("ws://{address}"), events)
}

fn request(agent: &PublicKey, name: &str) -> ConnectManagedAgentRequest {
    ConnectManagedAgentRequest {
        agent_pubkey: agent.to_bech32().unwrap(),
        name: name.to_string(),
        respond_to: RespondTo::Anyone,
        respond_to_allowlist: Vec::new(),
    }
}

#[tokio::test]
async fn connect_publishes_one_policy_and_reconnecting_keeps_it() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let (relay_url, events) = loopback_relay().await;
    let owner = Keys::generate();
    let agent = Keys::generate().public_key();
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = owner.clone();
    *state.relay_url_override.lock().unwrap() = Some(relay_url.clone());
    let dir = tempfile::tempdir().unwrap();
    let scope = RetentionScope {
        db_path: dir.path().join("retention.sqlite3"),
        relay_url,
        owner_keys: owner.clone(),
    };

    let connected = connect_agent_at(&state, &scope, request(&agent, "Hex"))
        .await
        .unwrap();

    assert_eq!(connected.policy_sync_error, None);
    assert!(!connected.policy_kept);
    assert_eq!(
        buzz_sdk_pkg::nip_oa::verify_auth_tag(&connected.auth_tag, &agent).unwrap(),
        owner.public_key()
    );
    let published = events.lock().unwrap().clone();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].pubkey, owner.public_key());
    assert_eq!(
        published[0].tags.identifier(),
        Some(agent.to_hex().as_str())
    );
    assert_eq!(
        managed_agent_content_from_event(&published[0]).unwrap(),
        connected.policy
    );

    let reconnected = connect_agent_at(&state, &scope, request(&agent, "Renamed"))
        .await
        .unwrap();

    assert!(reconnected.policy_kept);
    assert_eq!(reconnected.policy.name, "Hex");
    assert_eq!(events.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn connect_refuses_another_owners_agent_before_publishing() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let (relay_url, events) = loopback_relay().await;
    let owner = Keys::generate();
    let agent = Keys::generate();
    events
        .lock()
        .unwrap()
        .push(profile_presenting_tag(&Keys::generate(), &agent));
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = owner.clone();
    *state.relay_url_override.lock().unwrap() = Some(relay_url.clone());
    let dir = tempfile::tempdir().unwrap();
    let scope = RetentionScope {
        db_path: dir.path().join("retention.sqlite3"),
        relay_url,
        owner_keys: owner,
    };

    assert!(
        connect_agent_at(&state, &scope, request(&agent.public_key(), "Hex"))
            .await
            .is_err()
    );
    assert_eq!(events.lock().unwrap().len(), 1);
}
