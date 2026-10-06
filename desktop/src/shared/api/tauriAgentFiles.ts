import { invokeTauri } from "@/shared/api/tauri";

// ── NIP-AF agent files ──────────────────────────────────────────────────────

/**
 * One file an agent shares with its owner. `content` is `null` when the
 * agent lists the file without its text (not UTF-8, or too large to fit an
 * encrypted record).
 */
export type AgentFileEntry = {
  path: string;
  sha256: string;
  size: number;
  content: string | null;
  eventId: string;
  /** Unix seconds. */
  createdAt: number;
};

export type AgentFileEditStatus =
  | "pending"
  | "applied"
  | "conflict"
  | "declined";

/**
 * One of the owner's edit requests and the agent's answer. `sha256` is the
 * file's hash after the answer: the new version when applied, the agent's
 * current version on conflict.
 */
export type AgentFileEdit = {
  requestId: string;
  path: string;
  baseSha256: string;
  content: string;
  /** Unix seconds. */
  createdAt: number;
  status: AgentFileEditStatus;
  sha256: string | null;
  reason: string | null;
  answeredAt: number | null;
};

/**
 * Response shape for `get_agent_files`. `files` is sorted by path, `edits`
 * newest first. `truncated` flags a relay cap hit.
 */
export type AgentFilesListing = {
  files: AgentFileEntry[];
  edits: AgentFileEdit[];
  truncated: boolean;
  fetchedAt: number;
};

/** Owner-gated listing of an agent's shared files and the owner's edits. */
export async function getAgentFiles(
  agentPubkey: string,
): Promise<AgentFilesListing> {
  return invokeTauri<AgentFilesListing>("get_agent_files", { agentPubkey });
}

/**
 * Propose a full replacement for a shared file, based on the version the
 * owner edited. Resolves to the request's event id; the agent's answer shows
 * up in a later `getAgentFiles` listing.
 */
export async function proposeAgentFileEdit(input: {
  agentPubkey: string;
  path: string;
  baseSha256: string;
  content: string;
}): Promise<string> {
  return invokeTauri<string>("propose_agent_file_edit", input);
}
