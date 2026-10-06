import type { ConnectedAgentHandoff } from "@/shared/api/tauriManagedAgents";

function shellQuote(value: string) {
  return `'${value.replaceAll("'", `'\\''`)}'`;
}

/** Environment the agent's launcher sets beside its own BUZZ_PRIVATE_KEY. */
export function connectedAgentEnv({
  authTag,
  policy,
  relayUrl,
}: Pick<ConnectedAgentHandoff, "authTag" | "policy" | "relayUrl">) {
  return [
    `BUZZ_RELAY_URL=${relayUrl}`,
    `BUZZ_AUTH_TAG=${shellQuote(authTag)}`,
    `BUZZ_ACP_RESPOND_TO=${policy.respondTo}`,
    ...(policy.respondTo === "allowlist"
      ? [`BUZZ_ACP_RESPOND_TO_ALLOWLIST=${policy.respondToAllowlist.join(",")}`]
      : []),
    "BUZZ_ACP_RELAY_OBSERVER=true",
  ].join("\n");
}

/** Publishes the agent's profile carrying the tag, which is what makes it show as yours. */
export function connectedAgentProfileCommand({
  policy,
}: Pick<ConnectedAgentHandoff, "policy">) {
  return `buzz users set-profile --name ${shellQuote(policy.name)}`;
}

export function looksLikeSecretKey(input: string) {
  return input.trim().toLowerCase().startsWith("nsec1");
}
