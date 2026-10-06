import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  getAgentFiles,
  proposeAgentFileEdit,
  type AgentFilesListing,
} from "@/shared/api/tauriAgentFiles";
import { buildFileTree, type FileTreeNode } from "./lib/buildFileTree";

export const agentFilesQueryKey = (agentPubkey: string) =>
  ["agent-files", agentPubkey.toLowerCase()] as const;

/** How often to re-read while an edit request waits for the agent's answer. */
const PENDING_EDIT_POLL_MS = 3_000;

/**
 * Fetch + decrypt one agent's shared files and the owner's edit requests.
 * Owner-gated at the Rust layer, like {@link useAgentMemoryQuery}. Polls
 * while any edit is pending so its answer shows up without a manual refresh.
 */
export function useAgentFilesQuery(
  agentPubkey: string | null | undefined,
  options?: { enabled?: boolean },
) {
  const enabled = (options?.enabled ?? true) && !!agentPubkey;
  return useQuery<AgentFilesListing>({
    enabled,
    queryKey: agentFilesQueryKey(agentPubkey ?? ""),
    queryFn: () => getAgentFiles(agentPubkey as string),
    staleTime: 30_000,
    refetchInterval: (query) =>
      query.state.data?.edits.some((edit) => edit.status === "pending")
        ? PENDING_EDIT_POLL_MS
        : false,
  });
}

/** {@link useAgentFilesQuery} plus the listing shaped as a folder tree. */
export function useAgentFileTree(
  agentPubkey: string | null | undefined,
  options?: { enabled?: boolean },
): {
  query: ReturnType<typeof useAgentFilesQuery>;
  tree: FileTreeNode[] | null;
} {
  const query = useAgentFilesQuery(agentPubkey, options);
  const tree = React.useMemo(
    () => (query.data ? buildFileTree(query.data.files) : null),
    [query.data],
  );
  return { query, tree };
}

/** Send an edit request, then re-read so it shows as pending. */
export function useProposeAgentFileEdit(agentPubkey: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: {
      path: string;
      baseSha256: string;
      content: string;
    }) => proposeAgentFileEdit({ agentPubkey, ...input }),
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: agentFilesQueryKey(agentPubkey),
      }),
  });
}
