import { invoke } from "@tauri-apps/api/core";

import {
  BUILDERLAB_BACKEND_ORIGIN,
  deletionResponseDisposition,
  publicDeletionRequest,
  type CommunityDeletionTransport,
  type PendingCommunityDeletion,
} from "@/features/communities/communityDeletionPending";
import type {
  BuilderlabAuth,
  HostedCommunity,
} from "@/features/communities/hostedCommunityApi";

type RefreshContext = {
  accepted: Map<string, PendingCommunityDeletion>;
  isCurrent: () => boolean;
  ownerPubkey: () => string | null;
  identityMismatch: boolean;
  loadAccount: () => Promise<HostedCommunity[]>;
  setAuth: (auth: BuilderlabAuth | null) => void;
  clearAccount: () => void;
  restoreListed: (listed: HostedCommunity[]) => void;
};

/** Recheck accepted requests only during an explicit owner-list refresh. */
export async function refreshAcceptedCommunityDeletions(
  context: RefreshContext,
): Promise<void> {
  if (!context.isCurrent()) return;
  const skipReplay = context.identityMismatch;
  let auth = await invoke<BuilderlabAuth | null>("get_builderlab_auth");
  if (!context.isCurrent()) return;
  context.setAuth(auth);
  if (!auth) {
    context.clearAccount();
    return;
  }
  const listed = await context.loadAccount();
  if (!context.isCurrent() || skipReplay) return;
  if (auth.canDeleteBuzzCommunities !== true) return;

  let restored = false;
  try {
    for (const [communityId, envelope] of [...context.accepted.entries()]) {
      if (!listed.some((community) => community.id === communityId)) continue;
      const stillEligible = () =>
        context.isCurrent() &&
        context.ownerPubkey() === envelope.bound_owner_pubkey &&
        envelope.backend_origin === BUILDERLAB_BACKEND_ORIGIN &&
        context.accepted.get(communityId) === envelope;
      if (!stillEligible()) return;
      auth = await invoke<BuilderlabAuth | null>("get_builderlab_auth");
      if (!stillEligible()) return;
      context.setAuth(auth);
      if (!auth) {
        context.clearAccount();
        return;
      }
      if (auth.canDeleteBuzzCommunities !== true) return;
      const request = publicDeletionRequest(envelope);
      const response = await invoke<CommunityDeletionTransport>(
        "delete_builderlab_community",
        {
          communityId: request.community_id,
          host: request.host,
          requestId: request.request_id,
          acknowledgementVersion: request.acknowledgement_version,
        },
      );
      if (!stillEligible()) return;
      const disposition = deletionResponseDisposition(
        response,
        envelope,
        "check",
      );
      if (disposition === "abort") {
        context.accepted.delete(communityId);
        restored = true;
      } else if (disposition !== "accept") {
        throw new Error("Couldn't check deletion status.");
      }
    }
  } finally {
    if (restored && context.isCurrent()) context.restoreListed(listed);
  }
}
