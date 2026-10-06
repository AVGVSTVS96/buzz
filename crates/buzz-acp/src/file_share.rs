//! NIP-AF agent files: publish the files this agent shares with its owner and
//! answer the owner's edit requests. See `docs/nips/NIP-AF.md`.
//!
//! One task does everything on a fixed tick: rescan the shared paths, publish
//! whatever differs from the relay's heads, then answer pending edit
//! requests. Local changes and owner edits are therefore never applied
//! concurrently.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use buzz_core::agent_files::{
    build_edit_result, build_file_event, decrypt_edit_request, monotonic_created_at,
    validate_and_decrypt, validate_path, Body, EditRequest, EditResult, EditStatus,
    NIP44_PLAINTEXT_MAX,
};
use buzz_core::kind::{KIND_AGENT_FILE, KIND_AGENT_FILE_EDIT_REQUEST, KIND_AGENT_FILE_EDIT_RESULT};
use nostr::{Alphabet, Event, EventId, Filter, Keys, Kind, PublicKey, SingleLetterTag};
use sha2::{Digest, Sha256};

use crate::relay::RestClient;

/// How often shared paths are rescanned and edit requests polled.
const TICK: Duration = Duration::from_secs(5);
/// Ceiling for the retry delay after a failed tick.
const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// Shared files beyond this many are not published.
const MAX_FILES: usize = 1000;
/// File records published per tick, so a large initial share stays well
/// inside the relay's per-agent write quota it shares with chat.
const MAX_PUBLISHES_PER_TICK: usize = 5;
/// Page size for head and edit-request queries.
const QUERY_LIMIT: usize = 1000;
/// Owner clocks may lag ours; re-query this far behind the newest request.
const REQUEST_SKEW_SECS: u64 = 600;
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Resolve `--share` entries against the working directory, refusing any that
/// leave it. Entries are resolved lexically so a path that doesn't exist yet
/// can still be shared once it appears; symlinks are checked at scan time.
pub(crate) fn resolve_shared_paths(
    cwd: &Path,
    entries: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    entries
        .iter()
        .map(|entry| {
            let mut resolved = cwd.to_path_buf();
            for component in entry.components() {
                match component {
                    Component::CurDir => {}
                    Component::ParentDir => {
                        resolved.pop();
                    }
                    other => resolved.push(other),
                }
            }
            if resolved.starts_with(cwd) {
                Ok(resolved)
            } else {
                Err(format!(
                    "--share {} is outside the working directory {}",
                    entry.display(),
                    cwd.display()
                ))
            }
        })
        .collect()
}

/// A file's path on the wire: relative to `cwd`, `/`-separated, and valid
/// under the NIP-AF path grammar.
fn wire_path(cwd: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(cwd).ok()?;
    let segments = relative
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let wire = segments.join("/");
    validate_path(&wire).is_ok().then_some(wire)
}

/// Every shared file, keyed by wire path. Directories are walked recursively,
/// skipping hidden and git-ignored entries; symlinks are never followed, and
/// a shared path whose real location leaves `cwd` is skipped.
fn scan(cwd: &Path, roots: &[PathBuf]) -> (BTreeMap<String, PathBuf>, bool) {
    let mut files = BTreeMap::new();
    for root in roots {
        let Ok(root) = root.canonicalize() else {
            continue;
        };
        if !root.starts_with(cwd) {
            tracing::debug!(root = %root.display(), "shared path resolves outside the working directory");
            continue;
        }
        let walk = ignore::WalkBuilder::new(&root)
            .require_git(false)
            .sort_by_file_name(|a, b| a.cmp(b))
            .build();
        for entry in walk.flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            match wire_path(cwd, entry.path()) {
                Some(path) => {
                    if files.len() == MAX_FILES && !files.contains_key(&path) {
                        return (files, true);
                    }
                    files.insert(path, entry.into_path());
                }
                None => {
                    tracing::debug!(path = %entry.path().display(), "skipping unshareable path")
                }
            }
        }
    }
    (files, false)
}

/// Describe a file's current bytes. Files too large to inline are hashed as a
/// stream instead of read into memory.
fn describe(path: String, file: &Path) -> std::io::Result<Body> {
    if std::fs::metadata(file)?.len() <= NIP44_PLAINTEXT_MAX as u64 {
        return Ok(Body::for_contents(path, &std::fs::read(file)?));
    }
    let mut reader = std::fs::File::open(file)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    let mut size = 0u64;
    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        size += read as u64;
    }
    Ok(Body::File {
        path,
        sha256: hex::encode(hasher.finalize()),
        size,
        content: None,
    })
}

/// Replace `file`'s contents in one rename, keeping its permissions.
fn write_atomically(file: &Path, contents: &[u8]) -> std::io::Result<()> {
    let permissions = std::fs::metadata(file)?.permissions();
    let dir = file.parent().unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(contents)?;
    temp.as_file().set_permissions(permissions)?;
    temp.persist(file).map_err(|e| e.error)?;
    Ok(())
}

/// The relay's head for one path: `sha256` is `None` for a tombstone.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Head {
    created_at: u64,
    sha256: Option<String>,
}

fn file_sha256(body: &Body) -> Option<&str> {
    match body {
        Body::File { sha256, .. } => Some(sha256),
        Body::Removed { .. } => None,
    }
}

/// Records to publish so the relay matches `local`: every new or changed file,
/// then a tombstone for every published path that is no longer shared.
fn plan(heads: &HashMap<String, Head>, local: &BTreeMap<String, Body>) -> Vec<Body> {
    let changed = local.values().filter(|body| {
        heads.get(body.path()).and_then(|h| h.sha256.as_deref()) != file_sha256(body)
    });
    let mut removed: Vec<Body> = heads
        .iter()
        .filter(|(path, head)| head.sha256.is_some() && !local.contains_key(*path))
        .map(|(path, _)| Body::Removed { path: path.clone() })
        .collect();
    removed.sort_by(|a, b| a.path().cmp(b.path()));
    changed.cloned().chain(removed).collect()
}

/// The file `request` may be written to, or the answer when the agent can't
/// apply it to the file as it is now.
fn check_edit<'a>(
    request: &EditRequest,
    current: Option<&'a LocalFile>,
) -> Result<&'a Path, EditResult> {
    let answer = |status, sha256: Option<&str>, reason: Option<&str>| EditResult {
        status,
        path: request.path.clone(),
        sha256: sha256.map(str::to_string),
        reason: reason.map(str::to_string),
    };
    match current.map(|local| (&local.file, &local.body)) {
        Some((
            file,
            Body::File {
                sha256,
                content: Some(_),
                ..
            },
        )) if *sha256 == request.base_sha256 => Ok(file),
        Some((
            _,
            Body::File {
                sha256,
                content: Some(_),
                ..
            },
        )) => Err(answer(EditStatus::Conflict, Some(sha256), None)),
        Some((_, Body::File { content: None, .. })) => Err(answer(
            EditStatus::Declined,
            None,
            Some("file is too large or not text"),
        )),
        _ => Err(answer(
            EditStatus::Declined,
            None,
            Some("file is not shared"),
        )),
    }
}

struct LocalFile {
    file: PathBuf,
    stamp: (u64, Option<SystemTime>),
    body: Body,
}

/// Publishes the agent's shared files and answers the owner's edit requests.
pub(crate) struct FileShare {
    rest: RestClient,
    keys: Keys,
    owner: PublicKey,
    cwd: PathBuf,
    roots: Vec<PathBuf>,
    loaded: bool,
    heads: HashMap<String, Head>,
    local: BTreeMap<String, LocalFile>,
    truncated: bool,
    answered: HashSet<EventId>,
    unsent: HashMap<EventId, EditResult>,
    request_since: u64,
}

impl FileShare {
    pub(crate) fn new(
        rest: RestClient,
        keys: Keys,
        owner: PublicKey,
        cwd: PathBuf,
        roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            rest,
            keys,
            owner,
            cwd,
            roots,
            loaded: false,
            heads: HashMap::new(),
            local: BTreeMap::new(),
            truncated: false,
            answered: HashSet::new(),
            unsent: HashMap::new(),
            request_since: 0,
        }
    }

    /// Tick forever, backing off while the relay keeps failing.
    pub(crate) async fn run(mut self) {
        let mut delay = TICK;
        loop {
            delay = match self.tick().await {
                Ok(()) => TICK,
                Err(error) => {
                    let next = (delay * 2).min(MAX_BACKOFF);
                    tracing::warn!(target: "agent_files", retry_in_secs = next.as_secs(), "file share tick failed: {error}");
                    next
                }
            };
            tokio::time::sleep(delay).await;
        }
    }

    async fn tick(&mut self) -> Result<(), String> {
        if !self.loaded {
            self.heads = self.load_heads().await?;
            self.answered = self.load_answered().await?;
            self.loaded = true;
        }
        self.rescan();
        let published = self.publish_changes().await;
        let answered = self.answer_edit_requests().await;
        published.and(answered)
    }

    async fn publish_changes(&mut self) -> Result<(), String> {
        let bodies: BTreeMap<String, Body> = self
            .local
            .iter()
            .map(|(path, local)| (path.clone(), local.body.clone()))
            .collect();
        for body in plan(&self.heads, &bodies)
            .iter()
            .take(MAX_PUBLISHES_PER_TICK)
        {
            self.publish(body).await?;
        }
        Ok(())
    }

    fn rescan(&mut self) {
        let (files, truncated) = scan(&self.cwd, &self.roots);
        if truncated && !self.truncated {
            tracing::warn!(target: "agent_files", limit = MAX_FILES, "too many shared files; the rest are not published");
        }
        self.truncated = truncated;
        let mut local = BTreeMap::new();
        for (path, file) in files {
            let Ok(meta) = std::fs::metadata(&file) else {
                continue;
            };
            let stamp = (meta.len(), meta.modified().ok());
            let cached = self
                .local
                .remove(&path)
                .filter(|cached| cached.stamp == stamp && cached.file == file);
            let body = match cached {
                Some(cached) => cached.body,
                None => match describe(path.clone(), &file) {
                    Ok(body) => body,
                    Err(error) => {
                        tracing::debug!(target: "agent_files", path, "unreadable shared file: {error}");
                        continue;
                    }
                },
            };
            local.insert(path, LocalFile { file, stamp, body });
        }
        self.local = local;
    }

    async fn load_heads(&self) -> Result<HashMap<String, Head>, String> {
        let me = self.keys.public_key();
        let filter = Filter::new()
            .kind(Kind::Custom(KIND_AGENT_FILE as u16))
            .author(me)
            .custom_tags(
                SingleLetterTag::lowercase(Alphabet::P),
                [self.owner.to_hex()],
            )
            .limit(QUERY_LIMIT);
        let mut newest: HashMap<String, Head> = HashMap::new();
        for event in self.query(filter).await? {
            let Ok(body) = validate_and_decrypt(
                &event,
                &me,
                &self.owner,
                self.keys.secret_key(),
                &self.owner,
            ) else {
                continue;
            };
            let head = Head {
                created_at: event.created_at.as_secs(),
                sha256: file_sha256(&body).map(str::to_string),
            };
            let path = body.path().to_string();
            if newest
                .get(&path)
                .is_none_or(|h| h.created_at < head.created_at)
            {
                newest.insert(path, head);
            }
        }
        Ok(newest)
    }

    async fn load_answered(&self) -> Result<HashSet<EventId>, String> {
        let results = Filter::new()
            .kind(Kind::Custom(KIND_AGENT_FILE_EDIT_RESULT as u16))
            .author(self.keys.public_key())
            .custom_tags(
                SingleLetterTag::lowercase(Alphabet::P),
                [self.owner.to_hex()],
            )
            .limit(QUERY_LIMIT);
        Ok(self
            .query(results)
            .await?
            .iter()
            .flat_map(|event| event.tags.event_ids().copied())
            .collect())
    }

    async fn publish(&mut self, body: &Body) -> Result<(), String> {
        let created_at = monotonic_created_at(
            unix_now(),
            self.heads.get(body.path()).map(|h| h.created_at),
        );
        let event = build_file_event(&self.keys, &self.owner, body, created_at)
            .map_err(|e| e.to_string())?;
        submit(&self.rest, &event).await?;
        tracing::debug!(target: "agent_files", path = body.path(), removed = body.is_tombstone(), "published agent file");
        self.heads.insert(
            body.path().to_string(),
            Head {
                created_at,
                sha256: file_sha256(body).map(str::to_string),
            },
        );
        Ok(())
    }

    async fn answer_edit_requests(&mut self) -> Result<(), String> {
        let requests = Filter::new()
            .kind(Kind::Custom(KIND_AGENT_FILE_EDIT_REQUEST as u16))
            .author(self.owner)
            .custom_tags(
                SingleLetterTag::lowercase(Alphabet::P),
                [self.keys.public_key().to_hex()],
            )
            .since(self.request_since.into())
            .limit(QUERY_LIMIT);
        let mut requests = self.query(requests).await?;
        requests.sort_by_key(|event| event.created_at);
        for event in requests {
            if !self.answered.contains(&event.id) {
                self.answer(&event).await?;
            }
            self.request_since = self
                .request_since
                .max(event.created_at.as_secs().saturating_sub(REQUEST_SKEW_SECS));
        }
        Ok(())
    }

    /// Decide, apply and reply to one request. A decided answer whose reply
    /// fails to publish is kept and re-sent, never re-decided.
    async fn answer(&mut self, event: &Event) -> Result<(), String> {
        let result = match self.unsent.remove(&event.id) {
            Some(result) => result,
            None => {
                let me = self.keys.public_key();
                match decrypt_edit_request(
                    event,
                    &self.owner,
                    &me,
                    self.keys.secret_key(),
                    &self.owner,
                ) {
                    Ok(request) => self.apply(&request).await,
                    Err(error) => {
                        tracing::warn!(target: "agent_files", request = %event.id, "unreadable edit request: {error}");
                        self.answered.insert(event.id);
                        return Ok(());
                    }
                }
            }
        };
        let reply = build_edit_result(&self.keys, &self.owner, &event.id, &result)
            .map_err(|e| e.to_string())?;
        if let Err(error) = submit(&self.rest, &reply).await {
            self.unsent.insert(event.id, result);
            return Err(error);
        }
        tracing::info!(target: "agent_files", request = %event.id, path = result.path, status = ?result.status, "answered edit request");
        self.answered.insert(event.id);
        Ok(())
    }

    /// Apply an owner's edit if the file still matches its base, publishing
    /// the new record right away. Returns the answer to send.
    async fn apply(&mut self, request: &EditRequest) -> EditResult {
        let file = match check_edit(request, self.local.get(&request.path)) {
            Ok(file) => file.to_path_buf(),
            Err(answer) => return answer,
        };
        let written = write_atomically(&file, request.content.as_bytes())
            .and_then(|()| describe(request.path.clone(), &file));
        let body = match written {
            Ok(body) => body,
            Err(error) => {
                return EditResult {
                    status: EditStatus::Declined,
                    path: request.path.clone(),
                    sha256: None,
                    reason: Some(format!("write failed: {error}")),
                }
            }
        };
        let sha256 = file_sha256(&body).map(str::to_string);
        if let Err(error) = self.publish(&body).await {
            tracing::warn!(target: "agent_files", path = request.path, "edited file not yet published: {error}");
        }
        if let (Some(local), Ok(meta)) =
            (self.local.get_mut(&request.path), std::fs::metadata(&file))
        {
            local.body = body;
            local.stamp = (meta.len(), meta.modified().ok());
        }
        EditResult {
            status: EditStatus::Applied,
            path: request.path.clone(),
            sha256,
            reason: None,
        }
    }

    async fn query(&self, filter: Filter) -> Result<Vec<Event>, String> {
        let value = self
            .rest
            .query(&[filter])
            .await
            .map_err(|e| format!("relay query failed: {e}"))?;
        let events = value
            .as_array()
            .ok_or("relay query returned non-array")?
            .iter()
            .filter_map(|v| serde_json::from_value::<Event>(v.clone()).ok())
            .filter(|event| event.verify().is_ok())
            .collect();
        Ok(events)
    }
}

async fn submit(rest: &RestClient, event: &Event) -> Result<(), String> {
    let response = tokio::time::timeout(SUBMIT_TIMEOUT, rest.submit_event(event))
        .await
        .map_err(|_| "publish timed out".to_string())?
        .map_err(|e| format!("publish failed: {e}"))?;
    if response.get("accepted").and_then(|v| v.as_bool()) == Some(false) {
        return Err(format!(
            "relay rejected {}: {}",
            event.id,
            response
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        ));
    }
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::agent_files::sha256_hex;

    fn file(path: &str, content: &str) -> Body {
        Body::for_contents(path.into(), content.as_bytes())
    }

    fn head(created_at: u64, content: Option<&str>) -> Head {
        Head {
            created_at,
            sha256: content.map(|c| sha256_hex(c.as_bytes())),
        }
    }

    fn workspace() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().canonicalize().unwrap();
        (dir, cwd)
    }

    fn write(cwd: &Path, path: &str, content: &str) {
        let file = cwd.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }

    #[test]
    fn resolve_shared_paths_keeps_entries_inside_the_working_directory() {
        let cwd = Path::new("/work/agent");
        let resolved = resolve_shared_paths(
            cwd,
            &[
                "PLANS".into(),
                "./notes/today.md".into(),
                "a/../b".into(),
                "/work/agent/abs.md".into(),
                ".".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            resolved,
            [
                cwd.join("PLANS"),
                cwd.join("notes/today.md"),
                cwd.join("b"),
                cwd.join("abs.md"),
                cwd.to_path_buf(),
            ]
        );
    }

    #[test]
    fn resolve_shared_paths_refuses_entries_outside_the_working_directory() {
        let cwd = Path::new("/work/agent");
        for outside in [
            "..",
            "../other",
            "a/../../other",
            "/etc/passwd",
            "/work/agentx",
        ] {
            let err = resolve_shared_paths(cwd, &[outside.into()]).unwrap_err();
            assert!(
                err.contains("outside the working directory"),
                "{outside}: {err}"
            );
        }
    }

    #[test]
    fn scan_lists_shared_files_by_wire_path() {
        let (_dir, cwd) = workspace();
        write(&cwd, "PLANS/a.md", "a");
        write(&cwd, "PLANS/deep/b.md", "b");
        write(&cwd, "PLANS/.hidden", "h");
        write(&cwd, "PLANS/.gitignore", "ignored.log\n");
        write(&cwd, "PLANS/ignored.log", "x");
        write(&cwd, "notes.md", "n");
        write(&cwd, "unshared.md", "u");
        write(&cwd, ".profile.md", "p");
        let roots = resolve_shared_paths(
            &cwd,
            &[
                "PLANS".into(),
                "notes.md".into(),
                ".profile.md".into(),
                "missing".into(),
            ],
        )
        .unwrap();
        let (files, truncated) = scan(&cwd, &roots);
        assert!(!truncated);
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            [".profile.md", "PLANS/a.md", "PLANS/deep/b.md", "notes.md"]
        );
        assert_eq!(files["notes.md"], cwd.join("notes.md"));
    }

    #[cfg(unix)]
    #[test]
    fn scan_never_follows_symlinks_out_of_the_working_directory() {
        let (_outside, outside) = workspace();
        write(&outside, "secret.txt", "s");
        let (_dir, cwd) = workspace();
        write(&cwd, "PLANS/a.md", "a");
        std::os::unix::fs::symlink(outside.join("secret.txt"), cwd.join("PLANS/link.txt")).unwrap();
        std::os::unix::fs::symlink(&outside, cwd.join("escape")).unwrap();
        let roots = resolve_shared_paths(&cwd, &["PLANS".into(), "escape".into()]).unwrap();
        let (files, _) = scan(&cwd, &roots);
        assert_eq!(files.keys().collect::<Vec<_>>(), ["PLANS/a.md"]);
    }

    #[test]
    fn scan_stops_at_the_file_limit() {
        let (_dir, cwd) = workspace();
        for i in 0..=MAX_FILES {
            write(&cwd, &format!("many/{i:04}.txt"), "");
        }
        let roots = resolve_shared_paths(&cwd, &["many".into()]).unwrap();
        let (files, truncated) = scan(&cwd, &roots);
        assert!(truncated);
        assert_eq!(files.len(), MAX_FILES);
    }

    #[test]
    fn describe_hashes_large_files_without_inlining_them() {
        let (_dir, cwd) = workspace();
        let big = "a".repeat(NIP44_PLAINTEXT_MAX + 1);
        write(&cwd, "big.log", &big);
        let body = describe("big.log".into(), &cwd.join("big.log")).unwrap();
        assert_eq!(
            body,
            Body::File {
                path: "big.log".into(),
                sha256: sha256_hex(big.as_bytes()),
                size: big.len() as u64,
                content: None,
            }
        );
    }

    #[test]
    fn plan_publishes_new_and_changed_files_and_tombstones_the_rest() {
        let heads = HashMap::from([
            ("same.md".to_string(), head(1, Some("same"))),
            ("changed.md".to_string(), head(1, Some("old"))),
            ("gone.md".to_string(), head(1, Some("gone"))),
            ("already-removed.md".to_string(), head(1, None)),
        ]);
        let local = BTreeMap::from([
            ("same.md".to_string(), file("same.md", "same")),
            ("changed.md".to_string(), file("changed.md", "new")),
            ("new.md".to_string(), file("new.md", "new")),
        ]);
        assert_eq!(
            plan(&heads, &local),
            [
                file("changed.md", "new"),
                file("new.md", "new"),
                Body::Removed {
                    path: "gone.md".into()
                },
            ]
        );
        assert!(plan(&HashMap::new(), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn check_edit_applies_conflicts_or_declines() {
        let request = EditRequest {
            path: "a.md".into(),
            base_sha256: sha256_hex(b"base"),
            content: "proposed".into(),
        };
        let local = |body| LocalFile {
            file: PathBuf::from("/work/a.md"),
            stamp: (0, None),
            body,
        };
        let base = local(file("a.md", "base"));
        assert_eq!(
            check_edit(&request, Some(&base)),
            Ok(Path::new("/work/a.md"))
        );

        let moved_on = local(file("a.md", "moved on"));
        let conflict = check_edit(&request, Some(&moved_on)).unwrap_err();
        assert_eq!(conflict.status, EditStatus::Conflict);
        assert_eq!(conflict.sha256, Some(sha256_hex(b"moved on")));

        let binary = local(Body::for_contents("a.md".into(), &[0xff, 0xfe]));
        assert_eq!(
            check_edit(&request, Some(&binary)).unwrap_err().status,
            EditStatus::Declined
        );
        let unshared = check_edit(&request, None).unwrap_err();
        assert_eq!(unshared.status, EditStatus::Declined);
        assert_eq!(unshared.path, "a.md");
        assert!(unshared.reason.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn write_atomically_replaces_contents_and_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, cwd) = workspace();
        write(&cwd, "run.sh", "old");
        let file = cwd.join("run.sh");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o750)).unwrap();
        write_atomically(&file, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o750
        );
        assert_eq!(
            std::fs::read_dir(&cwd).unwrap().count(),
            1,
            "no temp file left behind"
        );
    }
}
