import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { buildChannelAgentSessionCandidates } from "./useChannelAgentSessions.ts";
import { useActiveAgentPubkeys } from "../../messages/lib/useActiveAgentPubkeys.ts";

const relayAgents = ["unknown", "online", "away", "offline"].map((status) => ({
  pubkey: status,
  name: status,
  status,
  channelIds: [],
  channels: [],
}));

test("session projection retains unknown rather than manufacturing deployed status", () => {
  const candidates = buildChannelAgentSessionCandidates({
    managedAgents: [],
    relayAgents,
  });
  assert.deepEqual(
    candidates.map(({ status }) => status),
    ["unknown", "deployed", "deployed", "stopped"],
  );
});

test("active-agent lookup requires positive relay liveness evidence", () => {
  let active;
  function Probe() {
    active = useActiveAgentPubkeys([], relayAgents);
    return null;
  }
  renderToStaticMarkup(React.createElement(Probe));
  assert.deepEqual([...active], ["online", "away"]);
});

test("only the verified owner may stop a relay agent's turn", () => {
  const owner = "a".repeat(64);
  const candidates = buildChannelAgentSessionCandidates({
    currentPubkey: owner.toUpperCase(),
    managedAgents: [],
    relayAgents: [
      { ...relayAgents[1], pubkey: "owned", ownerPubkey: owner },
      { ...relayAgents[1], pubkey: "foreign", ownerPubkey: "b".repeat(64) },
      { ...relayAgents[1], pubkey: "ownerless", ownerPubkey: null },
    ],
  });
  assert.deepEqual(
    candidates.map(({ pubkey, canInterruptTurn }) => [
      pubkey,
      canInterruptTurn,
    ]),
    [
      ["owned", true],
      ["foreign", false],
      ["ownerless", false],
    ],
  );
});
