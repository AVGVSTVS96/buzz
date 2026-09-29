use super::*;

/// Builtin catalog defaults come from runtime metadata, not user-editable JSON.
#[test]
fn builtin_catalog_entries_expose_runtime_configuration_defaults() {
    use crate::managed_agents::custom_harnesses::registry_test_lock;
    let _path_guard = crate::managed_agents::lock_path_mutex();
    let _lock = registry_test_lock();
    let entries = discover_acp_runtimes_from(None, true);
    for runtime in KNOWN_ACP_RUNTIMES {
        let entry = entries.iter().find(|entry| entry.id == runtime.id).unwrap();
        assert!(
            entry.definition_env.is_empty(),
            "build fallbacks are not definition overrides"
        );
        assert_eq!(
            entry.configuration_defaults,
            runtime.configuration_defaults()
        );
    }
}
