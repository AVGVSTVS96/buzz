import {
  fromRawManagedAgent,
  invokeTauri,
  type RawManagedAgent,
} from "@/shared/api/tauri";
import type {
  ManagedAgent,
  ManagedAgentRuntimeStatus,
  RespondToMode,
} from "@/shared/api/types";

export async function startManagedAgent(
  pubkey: string,
  options?: {
    /** Tenant scope captured by the caller before its first await; the
     * backend fails closed before any spawn/deploy side effect when the
     * active community no longer matches. */
    expectedRelayUrl?: string;
    /** Signer identity captured with the relay scope; the backend fails
     * closed when the active workspace identity no longer matches. */
    expectedSignerPubkey?: string;
    /** Unix-seconds replay floor for a publish-first mention send: the
     * spawned harness's first REQ replays at least back to this moment, so
     * the already-published triggering message lands in its window however
     * long the spawn takes. Local spawns receive it as process env; provider
     * deploys carry it in the payload's launch.policy_env. */
    replayFloorUnix?: number;
  },
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>("start_managed_agent", {
    pubkey,
    expectedRelayUrl: options?.expectedRelayUrl ?? null,
    expectedSignerPubkey: options?.expectedSignerPubkey ?? null,
    replayFloorUnix: options?.replayFloorUnix ?? null,
  });
  return fromRawManagedAgent(response);
}

export async function stopManagedAgent(pubkey: string): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>("stop_managed_agent", {
    pubkey,
  });
  return fromRawManagedAgent(response);
}

export async function setManagedAgentStartOnAppLaunch(
  pubkey: string,
  startOnAppLaunch: boolean,
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>(
    "set_managed_agent_start_on_app_launch",
    {
      pubkey,
      startOnAppLaunch,
    },
  );
  return fromRawManagedAgent(response);
}

export async function setManagedAgentAutoRestart(
  pubkey: string,
  autoRestartOnConfigChange: boolean,
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>(
    "set_managed_agent_auto_restart",
    {
      pubkey,
      autoRestartOnConfigChange,
    },
  );
  return fromRawManagedAgent(response);
}

export async function listManagedAgentRuntimes(): Promise<
  ManagedAgentRuntimeStatus[]
> {
  return invokeTauri<ManagedAgentRuntimeStatus[]>(
    "list_managed_agent_runtimes",
  );
}

export async function startManagedAgentRuntime(
  pubkey: string,
  relayUrl: string,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("start_managed_agent_runtime", { pubkey, relayUrl });
}

export async function stopManagedAgentRuntime(
  pubkey: string,
  relayUrl: string,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("stop_managed_agent_runtime", { pubkey, relayUrl });
}

export async function restartManagedAgentRuntime(
  pubkey: string,
  relayUrl: string,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("restart_managed_agent_runtime", { pubkey, relayUrl });
}

export async function putManagedAgentRuntimeLifecycle(
  outerPubkey: string,
  payload: unknown,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("put_managed_agent_runtime_lifecycle", {
    outerPubkey,
    payload,
  });
}

export async function reconcileManagedAgentRuntimes(
  communities: readonly { relayUrl: string }[],
): Promise<ManagedAgentRuntimeStatus[]> {
  return invokeTauri("reconcile_managed_agent_runtimes", { communities });
}

export type ConnectedAgentPolicy = {
  name: string;
  respondTo: RespondToMode;
  respondToAllowlist: string[];
};

export type ConnectedAgentHandoff = {
  agentPubkey: string;
  authTag: string;
  relayUrl: string;
  policy: ConnectedAgentPolicy;
  policyKept: boolean;
  policySyncError: string | null;
};

type RawConnectedAgentHandoff = {
  agent_pubkey: string;
  auth_tag: string;
  relay_url: string;
  policy: {
    name: string;
    respond_to: RespondToMode;
    respond_to_allowlist?: string[];
  };
  policy_kept: boolean;
  policy_sync_error: string | null;
};

export async function connectManagedAgent(input: {
  agentPubkey: string;
  name: string;
  respondTo: RespondToMode;
  respondToAllowlist: string[];
}): Promise<ConnectedAgentHandoff> {
  const response = await invokeTauri<RawConnectedAgentHandoff>(
    "connect_managed_agent",
    { input },
  );
  return {
    agentPubkey: response.agent_pubkey,
    authTag: response.auth_tag,
    relayUrl: response.relay_url,
    policy: {
      name: response.policy.name,
      respondTo: response.policy.respond_to,
      respondToAllowlist: response.policy.respond_to_allowlist ?? [],
    },
    policyKept: response.policy_kept,
    policySyncError: response.policy_sync_error,
  };
}

export async function disconnectManagedAgent(
  agentPubkey: string,
): Promise<string | null> {
  return invokeTauri<string | null>("disconnect_managed_agent", {
    agentPubkey,
  });
}
