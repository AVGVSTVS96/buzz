import assert from "node:assert/strict";
import test from "node:test";

import {
  isOtherSetupAgent,
  isOwnedAgentNotManagedOnDevice,
  ownedAgentsRunningElsewhere,
} from "./otherSetupAgent.ts";

const OWNER = "a".repeat(64);
const AGENT = "b".repeat(64);

test("fails closed while the local managed directory is unresolved", () => {
  assert.equal(
    isOtherSetupAgent({
      agentDirectoriesReady: false,
      currentPubkey: OWNER,
      managedAgents: [],
      profileOwnerPubkey: OWNER,
      pubkey: AGENT,
      relayAgents: [],
    }),
    false,
  );
});

test("labels a viewer-owned identity as not managed on this device", () => {
  assert.equal(
    isOtherSetupAgent({
      agentDirectoriesReady: true,
      currentPubkey: OWNER,
      managedAgents: [],
      profileOwnerPubkey: OWNER,
      pubkey: AGENT,
      relayAgents: [],
    }),
    true,
  );
});

test("a locally managed provider is not labeled as another device", () => {
  assert.equal(
    isOtherSetupAgent({
      agentDirectoriesReady: true,
      currentPubkey: OWNER,
      managedAgents: [{ pubkey: AGENT, backend: { type: "provider" } }],
      profileOwnerPubkey: OWNER,
      pubkey: AGENT,
      relayAgents: [],
    }),
    false,
  );
});

for (const [name, overrides, expected] of [
  ["owned absent key", {}, true],
  ["loading local inventory", { localInventoryReady: false }, false],
  ["exact local provider record", { isLocallyManaged: true }, false],
  ["different owner", { ownerPubkey: "b".repeat(64) }, false],
  ["unknown ownership", { ownerPubkey: null }, false],
]) {
  test(`shared provenance: ${name}`, () => {
    assert.equal(
      isOwnedAgentNotManagedOnDevice({
        currentPubkey: "a".repeat(64),
        ownerPubkey: "A".repeat(64),
        localInventoryReady: true,
        isLocallyManaged: false,
        ...overrides,
      }),
      expected,
    );
  });
}

test("lists only viewer-owned relay agents without a local record", () => {
  const relayAgent = (pubkey, ownerPubkey) => ({
    pubkey,
    ownerPubkey,
    name: pubkey.slice(0, 4),
    agentType: "agent",
    channels: [],
    channelIds: [],
    capabilities: [],
    status: "unknown",
    respondTo: "owner-only",
    respondToAllowlist: [],
  });
  const LOCAL = "c".repeat(64);
  const FOREIGN = "d".repeat(64);
  const relayAgents = [
    relayAgent(AGENT, OWNER),
    relayAgent(LOCAL, OWNER),
    relayAgent(FOREIGN, "e".repeat(64)),
    relayAgent("f".repeat(64), null),
  ];

  const listed = ownedAgentsRunningElsewhere({
    currentPubkey: OWNER,
    localInventoryReady: true,
    managedAgents: [{ pubkey: LOCAL.toUpperCase() }],
    relayAgents,
  });

  assert.deepEqual(
    listed.map((agent) => agent.pubkey),
    [AGENT],
  );
  assert.deepEqual(
    ownedAgentsRunningElsewhere({
      currentPubkey: OWNER,
      localInventoryReady: false,
      managedAgents: [],
      relayAgents,
    }),
    [],
  );
});
