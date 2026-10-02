//! Exercise failed persistence after successful OAuth through the public API.
use super::*;
use buzz_agent::AgentError;

#[tokio::test]
async fn successful_grants_report_cache_failure_without_publishing_a_token() {
    for refresh in [false, true] {
        let stub = spawn_stub(false).await;
        let cache = TempDir::new().unwrap();
        let opener = ScriptedOpener::new(Script::Approve);
        let cfg = config(&stub, "/disco/a", cache.path());
        if refresh {
            seed_cache(
                &cfg,
                cache.path(),
                json!({
                    "access_token": "expired",
                    "refresh_token": "refresh-canary",
                    "expires_at": 1,
                }),
            );
        }
        let src = PkceOAuthTokenSource::new_with(cfg.clone(), Arc::new(opener.clone())).unwrap();
        let path = cache_file_path(&cfg, cache.path());
        if refresh {
            std::fs::remove_file(&path).unwrap();
        }
        // A directory at the final filename permits the lock and OAuth exchange,
        // but makes the production atomic rename fail on Unix and Windows.
        std::fs::create_dir_all(&path).unwrap();
        let intent = if refresh {
            AuthIntent::Headless
        } else {
            AuthIntent::UserInitiated
        };
        let err = src.acquire_with_intent(intent, None).await.unwrap_err();
        assert_eq!(err, AuthError::CacheUnavailable);
        assert_eq!(err.code(), "cache_unavailable");
        let error = AgentError::from(err);
        assert_eq!(error.json_rpc_code(), -32003);
        assert!(error.to_string().starts_with("llm credential storage: "));
        let AgentError::LlmCredentialStorage(message) = error else {
            panic!("storage failure must not be classified as authentication");
        };
        assert!(message.contains("could not securely save"), "{message}");
        assert!(!message.contains("could not reach"), "{message}");
        assert!(!message.contains("refresh-canary"));
        assert!(!message.contains(&cache.path().to_string_lossy().to_string()));
        assert_eq!(
            stub.refresh_grants.load(Ordering::SeqCst),
            u64::from(refresh)
        );
        assert_eq!(stub.code_grants.load(Ordering::SeqCst), u64::from(!refresh));
        let parent = path.parent().unwrap();
        assert!(
            std::fs::read_dir(parent).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            }),
            "failed atomic write leaked a temp file"
        );
        assert!(!path.with_extension("json.cooldown").exists());
        let attempt: serde_json::Value = serde_json::from_slice(
            &std::fs::read(attempt_sidecar_path(&cfg, cache.path())).unwrap(),
        )
        .unwrap();
        assert_eq!(attempt["result"], "cache_unavailable");

        // Fix the storage obstacle. This SAME source must obtain a new grant:
        // the failed candidate was not installed in its in-memory cell.
        std::fs::remove_dir(&path).unwrap();
        let token = src.acquire_with_intent(intent, None).await.unwrap();
        let expected = if refresh {
            "refreshed-token-2"
        } else {
            "browser-token-2"
        };
        assert_eq!(token, expected);
        assert_eq!(opener.call_count(), if refresh { 0 } else { 2 });
        let fresh = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener.clone())).unwrap();
        assert_eq!(
            fresh
                .acquire_with_intent(AuthIntent::Headless, None)
                .await
                .unwrap(),
            expected
        );
    }
}
