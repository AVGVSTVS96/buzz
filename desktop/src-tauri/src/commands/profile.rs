use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::LazyLock,
};

use buzz_core_pkg::PresenceStatus;
use serde_json::Value;
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    managed_agents::persona_events::monotonic_created_at,
    models::{ProfileInfo, SearchUsersResponse, UserNotesResponse, UsersBatchResponse},
    nostr_convert,
    relay::{
        query_relay, query_relay_at_with_keys, relay_api_base_url_with_override,
        relay_http_base_url, submit_event_at_with_keys,
    },
};

#[tauri::command]
pub async fn get_profile(state: State<'_, AppState>) -> Result<ProfileInfo, String> {
    let my_pubkey = current_pubkey_hex(&state)?;
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [0],
            "authors": [my_pubkey],
            "limit": 1
        })],
    )
    .await?;

    Ok(events
        .first()
        .map(nostr_convert::profile_info_from_event)
        .transpose()?
        .unwrap_or_else(|| empty_profile_info(&current_pubkey_hex_unwrap(&state))))
}

#[tauri::command]
pub async fn update_profile(
    display_name: Option<String>,
    name: Option<String>,
    avatar_url: Option<String>,
    about: Option<String>,
    nip05_handle: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProfileInfo, String> {
    update_profile_inner(display_name, name, avatar_url, about, nip05_handle, &state).await
}

async fn update_profile_inner(
    display_name: Option<String>,
    name: Option<String>,
    avatar_url: Option<String>,
    about: Option<String>,
    nip05_handle: Option<String>,
    state: &AppState,
) -> Result<ProfileInfo, String> {
    let signer = state.signing_keys()?;
    let my_pubkey = signer.public_key().to_hex();
    let api_base_url = relay_api_base_url_with_override(state)
        .trim_end_matches('/')
        .to_string();

    update_profile_with_scope(
        state,
        &api_base_url,
        &signer,
        &my_pubkey,
        ProfileUpdateFields {
            display_name: display_name.as_deref(),
            name: name.as_deref(),
            avatar_url: avatar_url.as_deref(),
            about: about.as_deref(),
            nip05_handle: nip05_handle.as_deref(),
            normalize_updates: false,
        },
        None,
    )
    .await
}

fn profile_update_lock(
    api_base_url: &str,
    expected_pubkey: &str,
) -> &'static tokio::sync::Mutex<()> {
    const PROFILE_UPDATE_LOCK_STRIPES: usize = 64;
    static PROFILE_UPDATE_LOCKS: LazyLock<[tokio::sync::Mutex<()>; PROFILE_UPDATE_LOCK_STRIPES]> =
        LazyLock::new(|| std::array::from_fn(|_| tokio::sync::Mutex::new(())));

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    api_base_url.trim_end_matches('/').hash(&mut hasher);
    expected_pubkey.to_lowercase().hash(&mut hasher);
    &PROFILE_UPDATE_LOCKS[(hasher.finish() as usize) % PROFILE_UPDATE_LOCK_STRIPES]
}

fn should_check_expected_avatar(avatar_url: Option<&str>) -> bool {
    normalized_profile_field(avatar_url).is_some()
}

fn assert_expected_avatar(
    current: &Value,
    expected_avatar_url: Option<&str>,
) -> Result<(), String> {
    let current_avatar_url = current.get("picture").and_then(Value::as_str);
    if normalized_avatar_url(current_avatar_url) != normalized_avatar_url(expected_avatar_url) {
        return Err("profile avatar changed before deferred save".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn update_profile_at_relay(
    relay_url: String,
    expected_pubkey: String,
    expected_avatar_url: Option<String>,
    avatar_url: Option<String>,
    display_name: Option<String>,
    name: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProfileInfo, String> {
    update_profile_at_relay_inner(
        relay_url,
        expected_pubkey,
        expected_avatar_url,
        avatar_url,
        display_name,
        name,
        &state,
    )
    .await
}

async fn update_profile_at_relay_inner(
    relay_url: String,
    expected_pubkey: String,
    expected_avatar_url: Option<String>,
    avatar_url: Option<String>,
    display_name: Option<String>,
    name: Option<String>,
    state: &AppState,
) -> Result<ProfileInfo, String> {
    let signer = capture_expected_signer(state, &expected_pubkey)?;

    let api_base_url = relay_http_base_url(&relay_url)
        .trim_end_matches('/')
        .to_string();

    update_profile_with_scope(
        state,
        &api_base_url,
        &signer,
        &expected_pubkey,
        ProfileUpdateFields {
            display_name: display_name.as_deref(),
            name: name.as_deref(),
            avatar_url: avatar_url.as_deref(),
            about: None,
            nip05_handle: None,
            normalize_updates: true,
        },
        Some(expected_avatar_url.as_deref()),
    )
    .await
}

struct ProfileUpdateFields<'a> {
    display_name: Option<&'a str>,
    name: Option<&'a str>,
    avatar_url: Option<&'a str>,
    about: Option<&'a str>,
    nip05_handle: Option<&'a str>,
    normalize_updates: bool,
}

async fn update_profile_with_scope(
    state: &AppState,
    api_base_url: &str,
    signer: &nostr::Keys,
    expected_pubkey: &str,
    fields: ProfileUpdateFields<'_>,
    expected_avatar_url: Option<Option<&str>>,
) -> Result<ProfileInfo, String> {
    if signer.public_key().to_hex() != expected_pubkey {
        return Err("profile identity changed before avatar save".to_string());
    }

    let filter = serde_json::json!({
        "kinds": [0],
        "authors": [expected_pubkey],
        "limit": 1
    });
    let profile_update_lock = profile_update_lock(api_base_url, expected_pubkey);
    let _profile_update_guard = profile_update_lock.lock().await;

    let prior_events = query_relay_at_with_keys(
        state,
        api_base_url,
        std::slice::from_ref(&filter),
        signer,
        None,
    )
    .await?;
    let prior_event = prior_events.first();
    let current: Value = prior_event
        .and_then(|event| serde_json::from_str::<Value>(&event.content).ok())
        .unwrap_or(Value::Null);
    if should_check_expected_avatar(fields.avatar_url) {
        if let Some(expected_avatar_url) = expected_avatar_url {
            assert_expected_avatar(&current, expected_avatar_url)?;
        }
    }

    let builder = build_profile_event_from_current(&current, fields, prior_event)?;
    submit_event_at_with_keys(builder, state, api_base_url, signer).await?;

    let events = query_relay_at_with_keys(state, api_base_url, &[filter], signer, None).await?;
    Ok(events
        .first()
        .map(nostr_convert::profile_info_from_event)
        .transpose()?
        .unwrap_or_else(|| empty_profile_info(expected_pubkey)))
}

fn build_profile_event_from_current(
    current: &Value,
    fields: ProfileUpdateFields<'_>,
    prior_event: Option<&nostr::Event>,
) -> Result<nostr::EventBuilder, String> {
    let display_name = profile_update_field(fields.display_name, fields.normalize_updates)
        .or_else(|| current.get("display_name").and_then(Value::as_str));
    let name = profile_update_field(fields.name, fields.normalize_updates)
        .or_else(|| current.get("name").and_then(Value::as_str));
    let picture = profile_update_field(fields.avatar_url, fields.normalize_updates)
        .or_else(|| current.get("picture").and_then(Value::as_str));
    let about = profile_update_field(fields.about, fields.normalize_updates)
        .or_else(|| current.get("about").and_then(Value::as_str));
    let nip05 = profile_update_field(fields.nip05_handle, fields.normalize_updates)
        .or_else(|| current.get("nip05").and_then(Value::as_str));

    Ok(
        events::build_profile(display_name, name, picture, about, nip05)?.custom_created_at(
            monotonic_created_at(prior_event.map(|event| event.created_at.as_secs() as i64)),
        ),
    )
}

fn profile_update_field(value: Option<&str>, normalize: bool) -> Option<&str> {
    if normalize {
        normalized_profile_field(value)
    } else {
        value
    }
}

fn normalized_profile_field(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn capture_expected_signer(state: &AppState, expected_pubkey: &str) -> Result<nostr::Keys, String> {
    let signer = state.signing_keys()?;
    if signer.public_key().to_hex() != expected_pubkey {
        return Err("profile identity changed before avatar save".to_string());
    }
    Ok(signer)
}

fn normalized_avatar_url(avatar_url: Option<&str>) -> Option<&str> {
    avatar_url.map(str::trim).filter(|value| !value.is_empty())
}

#[tauri::command]
pub async fn get_user_profile(
    pubkey: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProfileInfo, String> {
    let target = match pubkey {
        Some(pk) => pk,
        None => current_pubkey_hex(&state)?,
    };

    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [0],
            "authors": [target.clone()],
            "limit": 1
        })],
    )
    .await?;

    Ok(events
        .first()
        .map(nostr_convert::profile_info_from_event)
        .transpose()?
        .unwrap_or_else(|| empty_profile_info(&target)))
}

#[tauri::command]
pub async fn get_users_batch(
    pubkeys: Vec<String>,
    state: State<'_, AppState>,
) -> Result<UsersBatchResponse, String> {
    if pubkeys.is_empty() {
        return Ok(UsersBatchResponse {
            profiles: HashMap::new(),
            missing: Vec::new(),
        });
    }
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [0],
            "authors": pubkeys,
        })],
    )
    .await?;

    Ok(nostr_convert::users_batch_from_events(&events, &pubkeys))
}

#[tauri::command]
pub async fn get_user_notes(
    pubkey: String,
    limit: Option<u32>,
    before: Option<i64>,
    before_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<UserNotesResponse, String> {
    let _ = before_id; // pure-nostr filter does not use the id-based cursor
    let mut filter = serde_json::Map::new();
    filter.insert("kinds".to_string(), serde_json::json!([1]));
    filter.insert("authors".to_string(), serde_json::json!([pubkey]));
    filter.insert(
        "limit".to_string(),
        serde_json::json!(limit.unwrap_or(20).min(100)),
    );
    if let Some(t) = before {
        filter.insert("until".to_string(), serde_json::json!(t));
    }

    let events = query_relay(&state, &[Value::Object(filter)]).await?;
    Ok(nostr_convert::user_notes_from_events(&events))
}

fn build_user_search_filter(query: &str, limit: usize, page: u32) -> serde_json::Value {
    serde_json::json!({
        "kinds": [0],
        "search": query,
        "search_mode": "prefix",
        "limit": limit,
        "page": page,
    })
}

#[tauri::command]
pub async fn search_users(
    query: String,
    limit: Option<u32>,
    cursor: Option<String>,
    state: State<'_, AppState>,
) -> Result<SearchUsersResponse, String> {
    let trimmed = query.trim();
    let max = limit.unwrap_or(8).min(500) as usize;
    let page = cursor
        .as_deref()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1);

    if max == 0 {
        return Ok(SearchUsersResponse {
            users: Vec::new(),
            next_cursor: None,
        });
    }

    if trimmed.is_empty() {
        let events = query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [0],
                "limit": max,
                "page": page,
            })],
        )
        .await?;

        // Emit a real next page cursor when the relay returned a full page, so
        // the empty-query people directory can page past its first page (the
        // relay honors `page`→offset for this non-search kind:0 listing). The
        // raw `events.len()` is the correct fullness signal — `list_user_search_results`
        // dedupes/truncates, so its output length can undercount a full page.
        let mut response = nostr_convert::list_user_search_results(&events, max);
        if events.len() >= max {
            response.next_cursor = Some((page + 1).to_string());
        }
        return Ok(response);
    }

    // NIP-50 full-text search on kind:0 profiles. The relay's HTTP bridge
    // intercepts the `search` field on POST /query and routes to Postgres FTS
    // (see `crates/buzz-relay/src/api/bridge.rs::handle_bridge_search`),
    // so we get indexed, server-side search instead of fetching every kind:0
    // and scanning client-side. The old path was capped at 2000 kind:0 events
    // by the relay's HTTP bridge limit, which silently hid users on busy relays.
    //
    // We fetch one bounded page (bridge accepts up to 500) and re-rank that page
    // locally because the relay scores FTS rank against the whole kind:0 JSON
    // `content` blob, where a hit in `display_name` is not weighted any higher
    // than a substring hit in `about`. The caller can request later pages via the
    // cursor so the UI cap is only a page size, not a terminal directory ceiling.
    //
    // `search_mode: "prefix"` matters: every caller of this command is a
    // typeahead surface (member picker, @mention popup, DM recipient search,
    // topbar people results), so a partially typed name must match. Without it
    // the relay runs whole-word `websearch_to_tsquery` matching and "tyl"
    // returns zero results for "Tyler". Same bridge-only extension the topbar
    // message search uses (see `build_search_messages_filter`).
    let events = query_relay(&state, &[build_user_search_filter(trimmed, max, page)]).await?;

    let mut response = nostr_convert::rank_user_search_results(&events, trimmed, max);
    if events.len() >= max {
        response.next_cursor = Some((page + 1).to_string());
    }
    Ok(response)
}

#[tauri::command]
pub async fn get_presence(
    pubkeys: Vec<String>,
    state: State<'_, AppState>,
) -> Result<HashMap<String, PresenceStatus>, String> {
    if pubkeys.is_empty() {
        return Ok(HashMap::new());
    }

    // Presence is published as kind:20001 ephemeral events. Query the most
    // recent per author. Only a successful empty snapshot establishes absence;
    // transport/auth/storage failures must reject so consumers remain unknown.
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [20001],
            "authors": pubkeys,
        })],
    )
    .await?;

    let mut latest: HashMap<String, (u64, PresenceStatus)> = HashMap::new();
    for ev in &events {
        // Relay-synthesized presence events use a p-tag to identify the subject.
        // Self-signed presence events (live WS) use the event author directly.
        let pk = ev
            .tags
            .iter()
            .find_map(|t| {
                let s = t.as_slice();
                if s.len() >= 2 && s[0] == "p" {
                    Some(s[1].clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| ev.pubkey.to_hex());
        let ts = ev.created_at.as_secs();
        let status = match ev.content.trim() {
            "online" => PresenceStatus::Online,
            "away" => PresenceStatus::Away,
            "offline" => PresenceStatus::Offline,
            _ => continue,
        };
        match latest.get(&pk) {
            Some((prev_ts, _)) if *prev_ts >= ts => {}
            _ => {
                latest.insert(pk, (ts, status));
            }
        }
    }

    Ok(latest
        .into_iter()
        .map(|(pk, (_, status))| (pk, status))
        .collect())
}

fn current_pubkey_hex(state: &AppState) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|e| e.to_string())?;
    Ok(keys.public_key().to_hex())
}

fn current_pubkey_hex_unwrap(state: &AppState) -> String {
    current_pubkey_hex(state).unwrap_or_default()
}

fn empty_profile_info(pubkey: &str) -> ProfileInfo {
    ProfileInfo {
        pubkey: pubkey.to_string(),
        display_name: None,
        avatar_url: None,
        about: None,
        nip05_handle: None,
        owner_pubkey: None,
        has_profile_event: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State as AxumState, routing::post, Json, Router};
    use std::sync::Arc;
    use tokio::sync::{mpsc, oneshot};

    #[derive(Clone, Copy)]
    enum ProfileRelayGate {
        Query,
        Submit,
    }

    #[derive(Clone)]
    struct ProfileRelayState {
        current: Arc<tokio::sync::Mutex<Option<nostr::Event>>>,
        gate: Arc<tokio::sync::Mutex<Option<(ProfileRelayGate, oneshot::Receiver<()>)>>>,
        gate_started: mpsc::Sender<()>,
    }

    struct ProfileRelay {
        url: String,
        current: Arc<tokio::sync::Mutex<Option<nostr::Event>>>,
        gate_release: Option<oneshot::Sender<()>>,
        gate_started: mpsc::Receiver<()>,
        server: tokio::task::JoinHandle<()>,
    }

    impl Drop for ProfileRelay {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    fn profile_event(
        keys: &nostr::Keys,
        display_name: Option<&str>,
        name: Option<&str>,
        avatar_url: Option<&str>,
    ) -> nostr::Event {
        events::build_profile(display_name, name, avatar_url, None, None)
            .expect("build profile")
            .sign_with_keys(keys)
            .expect("sign profile")
    }

    fn profile_state(keys: &nostr::Keys) -> AppState {
        let state = crate::app_state::build_app_state();
        *state.keys.lock().expect("lock keys") = keys.clone();
        state
    }

    async fn profile_relay(
        initial: Option<nostr::Event>,
        gate: Option<ProfileRelayGate>,
    ) -> ProfileRelay {
        let current = Arc::new(tokio::sync::Mutex::new(initial));
        let (gate_started, started_rx) = mpsc::channel(2);
        let (gate_release, release_rx) = oneshot::channel();
        let state = ProfileRelayState {
            current: Arc::clone(&current),
            gate: Arc::new(tokio::sync::Mutex::new(gate.map(|gate| (gate, release_rx)))),
            gate_started,
        };
        let app = Router::new()
            .route(
                "/query",
                post(|AxumState(state): AxumState<ProfileRelayState>| async move {
                    let release = {
                        let mut gate = state.gate.lock().await;
                        if matches!(gate.as_ref().map(|(gate, _)| gate), Some(ProfileRelayGate::Query)) {
                            gate.take().map(|(_, release)| release)
                        } else {
                            None
                        }
                    };
                    if let Some(release) = release {
                        let _ = state.gate_started.send(()).await;
                        let _ = release.await;
                    }
                    let events = state
                        .current
                        .lock()
                        .await
                        .as_ref()
                        .cloned()
                        .into_iter()
                        .collect::<Vec<_>>();
                    Json(events)
                }),
            )
            .route(
                &["/ev", "ents"].concat(),
                post(
                    |AxumState(state): AxumState<ProfileRelayState>, Json(event): Json<nostr::Event>| async move {
                        assert!(event.verify().is_ok());
                        let release = {
                            let mut gate = state.gate.lock().await;
                            if matches!(gate.as_ref().map(|(gate, _)| gate), Some(ProfileRelayGate::Submit)) {
                                gate.take().map(|(_, release)| release)
                            } else {
                                None
                            }
                        };
                        if let Some(release) = release {
                            let _ = state.gate_started.send(()).await;
                            let _ = release.await;
                        }
                        let event_id = event.id.to_hex();
                        replace_profile_head(&state.current, event).await;
                        Json(serde_json::json!({"event_id": event_id, "accepted": true, "message": ""}))
                    },
                ),
            )
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind relay");
        let address = listener.local_addr().expect("relay address");
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        ProfileRelay {
            url: format!("http://{address}"),
            current,
            gate_release: gate.map(|_| gate_release),
            gate_started: started_rx,
            server,
        }
    }

    fn profile_state_for_relay(keys: &nostr::Keys, relay_url: &str) -> AppState {
        let state = profile_state(keys);
        *state
            .relay_url_override
            .lock()
            .expect("lock relay override") = Some(relay_url.to_string());
        state
    }

    async fn replace_profile_head(
        current: &tokio::sync::Mutex<Option<nostr::Event>>,
        candidate: nostr::Event,
    ) {
        let mut current = current.lock().await;
        let should_replace = current.as_ref().is_none_or(|head| {
            candidate.created_at > head.created_at
                || (candidate.created_at == head.created_at && candidate.id < head.id)
        });
        if should_replace {
            *current = Some(candidate);
        }
    }

    fn profile_content(event: &nostr::Event) -> Value {
        serde_json::from_str(&event.content).expect("profile json")
    }

    #[tokio::test]
    async fn deferred_avatar_expect_empty_succeeds_only_while_empty() {
        let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
        crate::relay_admission::reset_rate_limit_gate();
        let keys = nostr::Keys::generate();
        let state = profile_state(&keys);
        let pubkey = keys.public_key().to_hex();
        let relay = profile_relay(None, None).await;

        let saved = update_profile_at_relay_inner(
            relay.url.clone(),
            pubkey.clone(),
            None,
            Some("https://example.com/first.png".to_string()),
            None,
            None,
            &state,
        )
        .await
        .expect("empty baseline accepts first avatar");
        assert_eq!(
            saved.avatar_url.as_deref(),
            Some("https://example.com/first.png")
        );

        *relay.current.lock().await = Some(profile_event(
            &keys,
            Some("Newer"),
            None,
            Some("https://example.com/newer.png"),
        ));
        let error = match update_profile_at_relay_inner(
            relay.url.clone(),
            pubkey,
            None,
            Some("https://example.com/stale.png".to_string()),
            None,
            None,
            &state,
        )
        .await
        {
            Ok(_) => panic!("stale empty-baseline avatar save unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error, "profile avatar changed before deferred save");
        let final_event = relay.current.lock().await.clone().expect("profile head");
        assert_eq!(
            profile_content(&final_event)["picture"],
            "https://example.com/newer.png"
        );
        crate::relay_admission::reset_rate_limit_gate();
    }

    #[tokio::test]
    async fn deferred_avatar_expect_nonempty_still_compares_current_avatar() {
        let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
        crate::relay_admission::reset_rate_limit_gate();
        let keys = nostr::Keys::generate();
        let state = profile_state(&keys);
        let pubkey = keys.public_key().to_hex();
        let relay = profile_relay(
            Some(profile_event(
                &keys,
                Some("Existing"),
                None,
                Some("https://example.com/old.png"),
            )),
            None,
        )
        .await;

        let saved = update_profile_at_relay_inner(
            relay.url.clone(),
            pubkey.clone(),
            Some("https://example.com/old.png".to_string()),
            Some("https://example.com/new.png".to_string()),
            None,
            None,
            &state,
        )
        .await
        .expect("matching nonempty baseline accepts avatar");
        assert_eq!(
            saved.avatar_url.as_deref(),
            Some("https://example.com/new.png")
        );

        let error = match update_profile_at_relay_inner(
            relay.url.clone(),
            pubkey,
            Some("https://example.com/old.png".to_string()),
            Some("https://example.com/stale.png".to_string()),
            None,
            None,
            &state,
        )
        .await
        {
            Ok(_) => panic!("stale nonempty-baseline avatar save unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error, "profile avatar changed before deferred save");
        let final_event = relay.current.lock().await.clone().expect("profile head");
        assert_eq!(
            profile_content(&final_event)["picture"],
            "https://example.com/new.png"
        );
        crate::relay_admission::reset_rate_limit_gate();
    }

    #[tokio::test]
    async fn name_only_profile_update_preserves_avatar_without_expected_avatar() {
        let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
        crate::relay_admission::reset_rate_limit_gate();
        let keys = nostr::Keys::generate();
        let state = profile_state(&keys);
        let pubkey = keys.public_key().to_hex();
        let relay = profile_relay(
            Some(profile_event(
                &keys,
                Some("Old Name"),
                Some("old"),
                Some("https://example.com/avatar.png"),
            )),
            None,
        )
        .await;

        let saved = update_profile_at_relay_inner(
            relay.url.clone(),
            pubkey,
            None,
            None,
            Some("New Name".to_string()),
            Some("new".to_string()),
            &state,
        )
        .await
        .expect("name-only corporate profile save remains valid");
        assert_eq!(saved.display_name.as_deref(), Some("New Name"));
        assert_eq!(
            saved.avatar_url.as_deref(),
            Some("https://example.com/avatar.png")
        );
        let final_event = relay.current.lock().await.clone().expect("profile head");
        let content = profile_content(&final_event);
        assert_eq!(content["display_name"], "New Name");
        assert_eq!(content["name"], "new");
        assert_eq!(content["picture"], "https://example.com/avatar.png");
        crate::relay_admission::reset_rate_limit_gate();
    }

    #[tokio::test]
    async fn profile_update_lock_forces_late_deferred_avatar_to_recheck_after_newer_avatar() {
        let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
        crate::relay_admission::reset_rate_limit_gate();
        let keys = nostr::Keys::generate();
        let state = Arc::new(profile_state(&keys));
        let pubkey = keys.public_key().to_hex();
        let mut relay = profile_relay(None, Some(ProfileRelayGate::Submit)).await;

        let newer = tokio::spawn({
            let state = Arc::clone(&state);
            let relay_url = relay.url.clone();
            let pubkey = pubkey.clone();
            async move {
                update_profile_at_relay_inner(
                    relay_url,
                    pubkey,
                    None,
                    Some("https://example.com/newer.png".to_string()),
                    None,
                    None,
                    &state,
                )
                .await
            }
        });
        relay
            .gate_started
            .recv()
            .await
            .expect("newer save reached relay submit");

        let older = tokio::spawn({
            let state = Arc::clone(&state);
            let relay_url = relay.url.clone();
            let pubkey = pubkey.clone();
            async move {
                update_profile_at_relay_inner(
                    relay_url,
                    pubkey,
                    None,
                    Some("https://example.com/older.png".to_string()),
                    None,
                    None,
                    &state,
                )
                .await
            }
        });

        relay
            .gate_release
            .take()
            .expect("first submit release")
            .send(())
            .expect("release first submit");
        let newer_profile = newer.await.expect("newer task").expect("newer save");
        assert_eq!(
            newer_profile.avatar_url.as_deref(),
            Some("https://example.com/newer.png")
        );
        let older_error = match older.await.expect("older task") {
            Ok(_) => panic!("older deferred avatar save unexpectedly overwrote newer avatar"),
            Err(error) => error,
        };
        assert_eq!(older_error, "profile avatar changed before deferred save");
        let final_event = relay.current.lock().await.clone().expect("profile head");
        assert_eq!(
            profile_content(&final_event)["picture"],
            "https://example.com/newer.png"
        );
        crate::relay_admission::reset_rate_limit_gate();
    }

    #[tokio::test]
    async fn ordinary_profile_update_waits_for_deferred_query_and_wins_canonical_head() {
        let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
        crate::relay_admission::reset_rate_limit_gate();
        let keys = nostr::Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let future_created_at = nostr::Timestamp::now().as_secs() + 60;
        let existing = events::build_profile(
            Some("Existing"),
            Some("existing"),
            Some("https://example.com/old.png"),
            None,
            None,
        )
        .expect("build existing profile")
        .custom_created_at(nostr::Timestamp::from(future_created_at))
        .sign_with_keys(&keys)
        .expect("sign existing profile");
        let mut relay = profile_relay(Some(existing), Some(ProfileRelayGate::Query)).await;
        let state = Arc::new(profile_state_for_relay(&keys, &relay.url));

        let deferred = tokio::spawn({
            let state = Arc::clone(&state);
            let relay_url = relay.url.clone();
            let pubkey = pubkey.clone();
            async move {
                update_profile_at_relay_inner(
                    relay_url,
                    pubkey,
                    Some("https://example.com/old.png".to_string()),
                    Some("https://example.com/deferred.png".to_string()),
                    None,
                    None,
                    &state,
                )
                .await
            }
        });
        relay
            .gate_started
            .recv()
            .await
            .expect("deferred save reached held scoped query");

        let settings = tokio::spawn({
            let state = Arc::clone(&state);
            async move {
                update_profile_inner(
                    Some("Settings Name".to_string()),
                    None,
                    Some("https://example.com/settings.png".to_string()),
                    None,
                    None,
                    &state,
                )
                .await
            }
        });
        let mut settings = Box::pin(settings);
        tokio::select! {
            result = &mut settings => {
                let completed = result
                    .map(|profile| profile.map(|profile| profile.avatar_url))
                    .map_err(|error| error.to_string());
                panic!("ordinary Settings update completed before deferred query lock released: {completed:?}");
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
        }

        relay
            .gate_release
            .take()
            .expect("deferred query release")
            .send(())
            .expect("release deferred query");
        let deferred_profile = deferred
            .await
            .expect("deferred task")
            .expect("deferred save");
        assert_eq!(
            deferred_profile.avatar_url.as_deref(),
            Some("https://example.com/deferred.png")
        );
        let settings_profile = settings
            .await
            .expect("settings task")
            .expect("settings save");
        assert_eq!(
            settings_profile.display_name.as_deref(),
            Some("Settings Name")
        );
        assert_eq!(
            settings_profile.avatar_url.as_deref(),
            Some("https://example.com/settings.png")
        );

        let final_event = relay.current.lock().await.clone().expect("profile head");
        let final_content = profile_content(&final_event);
        assert_eq!(final_content["display_name"], "Settings Name");
        assert_eq!(final_content["picture"], "https://example.com/settings.png");
        assert_eq!(final_event.created_at.as_secs(), future_created_at + 2);
        crate::relay_admission::reset_rate_limit_gate();
    }

    #[test]
    fn deferred_profile_signer_is_captured_and_rejects_wrong_identity() {
        let state = crate::app_state::build_app_state();
        let original = state.signing_keys().expect("signable identity");
        let original_pubkey = original.public_key().to_hex();

        let captured = capture_expected_signer(&state, &original_pubkey)
            .expect("matching identity should be captured");
        *state.keys.lock().expect("lock keys") = nostr::Keys::generate();

        assert_eq!(captured.public_key().to_hex(), original_pubkey);
        assert_ne!(
            state.keys.lock().expect("lock keys").public_key().to_hex(),
            original_pubkey
        );
        assert_eq!(
            capture_expected_signer(&state, &original_pubkey).unwrap_err(),
            "profile identity changed before avatar save"
        );
    }

    #[test]
    fn deferred_profile_event_is_strictly_newer_than_prior_head() {
        let keys = nostr::Keys::generate();
        let prior_created_at = nostr::Timestamp::now().as_secs() + 60;
        let prior_event = nostr::EventBuilder::new(
            nostr::Kind::Metadata,
            serde_json::json!({"display_name": "Larry"}).to_string(),
        )
        .custom_created_at(nostr::Timestamp::from(prior_created_at))
        .sign_with_keys(&keys)
        .expect("sign prior profile");

        let builder = build_profile_event_from_current(
            &serde_json::json!({"display_name": "Larry"}),
            ProfileUpdateFields {
                display_name: None,
                name: None,
                avatar_url: Some("https://example.com/avatar.png"),
                about: None,
                nip05_handle: None,
                normalize_updates: true,
            },
            Some(&prior_event),
        )
        .expect("build deferred profile");
        let event = builder
            .sign_with_keys(&keys)
            .expect("sign deferred profile");

        assert_eq!(event.created_at.as_secs(), prior_created_at + 1);
        assert_eq!(
            serde_json::from_str::<Value>(&event.content).unwrap()["picture"],
            "https://example.com/avatar.png"
        );
    }

    #[test]
    fn user_search_filter_requests_prefix_mode_for_typeahead() {
        // Every caller of `search_users` is a typeahead surface. Whole-word
        // FTS matching returns zero results for a partially typed name
        // ("tyl" for "Tyler"), which reads as "user doesn't exist" in the
        // member picker and @mention popup. Pin the mode so it can't drift.
        let filter = build_user_search_filter("tyl", 25, 1);

        assert_eq!(filter["search"], serde_json::json!("tyl"));
        assert_eq!(filter["search_mode"], serde_json::json!("prefix"));
        assert_eq!(filter["limit"], serde_json::json!(25));
        assert_eq!(filter["page"], serde_json::json!(1));
    }
}

#[cfg(test)]
#[path = "profile_presence_tests.rs"]
mod presence_tests;
