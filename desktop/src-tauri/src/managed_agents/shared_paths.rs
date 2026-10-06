use std::collections::BTreeMap;

/// buzz-acp's `--share` list. Entries are relative to the harness working
/// directory; one resolving outside it is a startup config error.
const SHARE_ENV_VAR: &str = "BUZZ_ACP_SHARE";

/// Validate and normalize the paths an agent shares with its owner: trimmed,
/// blank entries dropped. [`SHARE_ENV_VAR`] is comma-delimited, so a path
/// containing a comma cannot be expressed and is rejected.
pub(crate) fn validate_shared_paths(input: &[String]) -> Result<Vec<String>, String> {
    input
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .map(|path| {
            if path.contains(',') {
                Err(format!("shared path '{path}' cannot contain a comma"))
            } else {
                Ok(path.to_string())
            }
        })
        .collect()
}

fn shared_paths_env_value(paths: &[String]) -> Option<String> {
    (!paths.is_empty()).then(|| paths.join(","))
}

pub(crate) fn apply_shared_paths_env(command: &mut std::process::Command, paths: &[String]) {
    match shared_paths_env_value(paths) {
        Some(value) => command.env(SHARE_ENV_VAR, value),
        None => command.env_remove(SHARE_ENV_VAR),
    };
}

pub(crate) fn insert_shared_paths_env(policy_env: &mut BTreeMap<String, String>, paths: &[String]) {
    if let Some(value) = shared_paths_env_value(paths) {
        policy_env.insert(SHARE_ENV_VAR.to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_share(command: &std::process::Command) -> Option<Option<&str>> {
        command
            .get_envs()
            .find(|(key, _)| *key == SHARE_ENV_VAR)
            .map(|(_, value)| value.and_then(std::ffi::OsStr::to_str))
    }

    #[test]
    fn validation_trims_and_drops_blank_entries() {
        let paths = validate_shared_paths(&[" PLANS ".into(), "".into(), "notes/today.md".into()])
            .unwrap_or_else(|error| panic!("paths should validate: {error}"));
        assert_eq!(paths, ["PLANS", "notes/today.md"]);
    }

    #[test]
    fn validation_rejects_a_path_the_env_var_cannot_express() {
        let error = validate_shared_paths(&["a,b.md".into()]).unwrap_err();
        assert!(error.contains("comma"), "{error}");
    }

    #[test]
    fn local_launch_env_receives_the_comma_joined_paths() {
        let mut command = std::process::Command::new("true");
        command.env(SHARE_ENV_VAR, "ambient");

        apply_shared_paths_env(&mut command, &["PLANS".into(), "notes/today.md".into()]);

        assert_eq!(command_share(&command), Some(Some("PLANS,notes/today.md")));
    }

    #[test]
    fn local_launch_without_shared_paths_strips_an_ambient_value() {
        let mut command = std::process::Command::new("true");
        command.env(SHARE_ENV_VAR, "ambient");

        apply_shared_paths_env(&mut command, &[]);

        assert_eq!(command_share(&command), Some(None));
    }

    #[test]
    fn provider_policy_env_carries_paths_only_when_present() {
        let mut policy_env = BTreeMap::new();
        insert_shared_paths_env(&mut policy_env, &[]);
        assert!(policy_env.is_empty());

        insert_shared_paths_env(&mut policy_env, &["PLANS".into(), "notes.md".into()]);
        assert_eq!(
            policy_env.get(SHARE_ENV_VAR).map(String::as_str),
            Some("PLANS,notes.md")
        );
    }
}
