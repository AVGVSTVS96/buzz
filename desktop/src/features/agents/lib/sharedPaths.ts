/**
 * Pure helpers for the "Shared files" agent setting, edited as one path per
 * line. The Rust side is the canonical validator (see
 * `desktop/src-tauri/src/managed_agents/shared_paths.rs`); these give inline
 * feedback before the round-trip.
 */

export const SHARED_PATHS_HELP =
  "Files or folders, one per line, relative to the agent's working folder (~/.buzz for agents on this computer). You can see them and suggest edits.";

export function parseSharedPaths(text: string): string[] {
  return text
    .split("\n")
    .map((path) => path.trim())
    .filter((path) => path.length > 0);
}

/** The harness receives the list comma-separated, so a path can't hold one. */
export function sharedPathsError(text: string): string | null {
  return parseSharedPaths(text).some((path) => path.includes(","))
    ? "Paths can't contain commas."
    : null;
}

/** The paths to save, or `undefined` when they match the saved list. */
export function sharedPathsUpdate(
  text: string,
  saved: readonly string[],
): string[] | undefined {
  const paths = parseSharedPaths(text);
  const unchanged =
    paths.length === saved.length &&
    paths.every((path, index) => path === saved[index]);
  return unchanged ? undefined : paths;
}
