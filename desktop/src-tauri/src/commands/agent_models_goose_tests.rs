use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn bundled_goose_discovery_preserves_file_and_selected_provider() {
    const CHILD: &str = "BUZZ_TEST_GOOSE_DISCOVERY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let root = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "commands::agent_models::goose_tests::bundled_goose_discovery_preserves_file_and_selected_provider", "--nocapture"])
            .env(CHILD, "1")
            .env("GOOSE_PATH_ROOT", root.path())
            .env_remove("GOOSE_PROVIDER")
            .env_remove("GOOSE_MODEL")
            .status().unwrap();
        assert!(status.success());
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("GOOSE_PATH_ROOT").unwrap());
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::write(
        root.join("config/config.yaml"),
        "GOOSE_PROVIDER: anthropic\nGOOSE_MODEL: file-model\n",
    )
    .unwrap();
    let record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": "test", "name": "test", "runtime": "goose",
        "private_key_nsec": "", "relay_url": "", "auth_tag": "",
        "acp_command": "buzz-acp", "agent_command": "goose",
        "agent_args": [], "mcp_command": "", "turn_timeout_seconds": 300,
        "parallelism": 1, "created_at": "", "updated_at": ""
    }))
    .unwrap();
    let saved = agent_model_discovery_config(&record, &[], &Default::default()).unwrap();
    let draft = draft_agent_model_discovery_env("goose", None, &BTreeMap::new(), &BTreeMap::new());
    for env in [&saved.env, &draft] {
        assert!(
            !env.contains_key("GOOSE_PROVIDER"),
            "file provider must remain authoritative"
        );
        assert!(
            !env.contains_key("GOOSE_MODEL"),
            "file model must remain authoritative"
        );
    }
    // Exercise the actual discovery child environment, where raw metadata used
    // to reinsert build defaults after descriptor resolution. No network needed.
    let probe = root.join("buzz-acp");
    std::fs::write(
        &probe,
        r#"#!/bin/sh
if [ "${GOOSE_PROVIDER+x}" ] || [ "${GOOSE_MODEL+x}" ]; then
  echo 'discovery injected provider/model above file configuration' >&2
  exit 1
fi
printf '%s\n' '{"unstable":{"availableModels":[{"modelId":"file-model"}]}}'
"#,
    )
    .unwrap();
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let models = run_agent_models_command(probe, saved.command, saved.args, None, saved.env)
        .await
        .unwrap();
    assert_eq!(models.models[0].id, "file-model");

    std::fs::write(root.join("config/config.yaml"), "{}\n").unwrap();
    let draft = draft_agent_model_discovery_env(
        "goose",
        Some("anthropic"),
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    assert_eq!(
        draft.get("GOOSE_PROVIDER").map(String::as_str),
        Some("anthropic")
    );
    assert!(!draft.contains_key("GOOSE_MODEL"));
    let defaults = known_acp_runtime("goose").unwrap().configuration_defaults();
    let draft = draft_agent_model_discovery_env("goose", None, &BTreeMap::new(), &BTreeMap::new());
    for (key, value) in defaults {
        assert_eq!(
            draft.get(&key),
            Some(&value),
            "fresh discovery uses build defaults"
        );
    }
}
