import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import test from "node:test";

import {
  connectedAgentEnv,
  connectedAgentProfileCommand,
  looksLikeSecretKey,
} from "./connectedAgentHandoff.ts";

const AUTH_TAG = JSON.stringify(["auth", "a".repeat(64), "", "b".repeat(128)]);

function handoff(policy) {
  return {
    authTag: AUTH_TAG,
    relayUrl: "wss://relay.example",
    policy: {
      name: "Hex",
      respondTo: "owner-only",
      respondToAllowlist: [],
      ...policy,
    },
  };
}

function sourceInShell(env, variable) {
  return execFileSync(
    "sh",
    ["-c", `set -a; eval "$ENV_LINES"; printf %s "$${variable}"`],
    { encoding: "utf8", env: { ENV_LINES: env } },
  );
}

test("the environment hands the exact tag to the agent's shell", () => {
  const env = connectedAgentEnv(handoff());

  assert.equal(sourceInShell(env, "BUZZ_AUTH_TAG"), AUTH_TAG);
  assert.equal(sourceInShell(env, "BUZZ_RELAY_URL"), "wss://relay.example");
  assert.equal(sourceInShell(env, "BUZZ_ACP_RESPOND_TO"), "owner-only");
  assert.equal(sourceInShell(env, "BUZZ_ACP_RELAY_OBSERVER"), "true");
  assert.doesNotMatch(env, /RESPOND_TO_ALLOWLIST/);
});

test("the environment carries the policy's allowlist", () => {
  const env = connectedAgentEnv(
    handoff({
      respondTo: "allowlist",
      respondToAllowlist: ["c".repeat(64), "d".repeat(64)],
    }),
  );

  assert.equal(
    sourceInShell(env, "BUZZ_ACP_RESPOND_TO_ALLOWLIST"),
    `${"c".repeat(64)},${"d".repeat(64)}`,
  );
});

test("the profile command keeps the agent's name intact", () => {
  const name = 'Bassim\'s "Hex" $HOME';
  const args = execFileSync(
    "sh",
    ["-c", `buzz() { printf '%s\\n' "$@"; }; eval "$COMMAND"`],
    {
      encoding: "utf8",
      env: { COMMAND: connectedAgentProfileCommand(handoff({ name })) },
    },
  );

  assert.deepEqual(args.trimEnd().split("\n"), [
    "users",
    "set-profile",
    "--name",
    name,
  ]);
});

test("an nsec is recognized as a secret key", () => {
  assert.equal(looksLikeSecretKey(" NSEC1qqqq "), true);
  assert.equal(looksLikeSecretKey("npub1qqqq"), false);
});
