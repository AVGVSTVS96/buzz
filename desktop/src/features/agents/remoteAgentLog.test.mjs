import assert from "node:assert/strict";
import { afterEach, beforeEach, mock, test } from "node:test";

import {
  _testProcessLiveObserverEvents,
  _testRegisterKnownAgents,
  getAgentObserverSnapshot,
  ingestArchivedObserverEvents,
  resetAgentObserverStore,
} from "./observerRelayStore.ts";
import {
  appendRemoteAgentLog,
  clearRemoteAgentLog,
  followRemoteAgentLog,
  getRemoteAgentLog,
} from "./remoteAgentLog.ts";

const AGENT = "a".repeat(64);
const OWNER = "b".repeat(64);

function observerEvent(seq, kind, payload, channelId = null) {
  return {
    seq,
    timestamp: `2026-10-06T12:00:0${seq}.000Z`,
    kind,
    agentIndex: null,
    channelId,
    sessionId: null,
    turnId: null,
    payload,
  };
}

beforeEach(() => {
  resetAgentObserverStore();
  clearRemoteAgentLog(AGENT);
});

afterEach(() => {
  mock.timers.reset();
});

test("live log events feed the agent log and stay out of the activity journal", () => {
  _testProcessLiveObserverEvents(AGENT, [
    observerEvent(1, "log", { lines: ["connected to relay"] }),
    observerEvent(2, "turn_started", {}, "chan-1"),
    observerEvent(3, "log", { lines: ["turn started"], dropped: 0 }),
  ]);

  assert.deepEqual(getRemoteAgentLog(AGENT), {
    answered: true,
    lines: ["connected to relay", "turn started"],
  });
  assert.deepEqual(
    getAgentObserverSnapshot(AGENT).events.map((event) => event.kind),
    ["turn_started"],
  );
});

test("archived log events are not replayed into activity or the log", async () => {
  _testRegisterKnownAgents("sub", [AGENT]);
  const raw = {
    id: "e".repeat(64),
    pubkey: AGENT,
    created_at: 1,
    kind: 24200,
    tags: [
      ["p", OWNER],
      ["agent", AGENT],
      ["frame", "telemetry"],
    ],
    content: "encrypted",
    sig: "s".repeat(128),
  };

  await ingestArchivedObserverEvents([raw], async () =>
    observerEvent(1, "log", { lines: ["stale line"] }),
  );

  assert.deepEqual(getAgentObserverSnapshot(AGENT).events, []);
  assert.equal(getRemoteAgentLog(AGENT).answered, false);
});

test("an empty answer counts as answered, and skipped lines leave a gap marker", () => {
  appendRemoteAgentLog(AGENT, { lines: [] });
  assert.deepEqual(getRemoteAgentLog(AGENT), { answered: true, lines: [] });

  appendRemoteAgentLog(AGENT, { lines: ["after the gap", 7], dropped: 3 });
  assert.deepEqual(getRemoteAgentLog(AGENT).lines, [
    "[3 log lines skipped]",
    "after the gap",
  ]);
});

test("the log keeps the newest 1000 lines", () => {
  appendRemoteAgentLog(AGENT, {
    lines: Array.from({ length: 1_005 }, (_, index) => `line ${index}`),
  });

  const { lines } = getRemoteAgentLog(AGENT);
  assert.equal(lines.length, 1_000);
  assert.equal(lines[0], "line 5");
  assert.equal(lines.at(-1), "line 1004");
});

test("following asks for a fresh tail, renews with no tail, and stops on cleanup", () => {
  mock.timers.enable({ apis: ["setInterval", "setTimeout"] });
  appendRemoteAgentLog(AGENT, { lines: ["from an earlier follow"] });
  const tails = [];
  const errors = [];

  const stop = followRemoteAgentLog(
    AGENT,
    200,
    (error) => errors.push(error),
    async (_pubkey, tail) => {
      tails.push(tail);
    },
  );
  assert.equal(getRemoteAgentLog(AGENT).answered, false);
  appendRemoteAgentLog(AGENT, { lines: ["tail"] });

  mock.timers.tick(60_000);
  stop();
  mock.timers.tick(60_000);

  assert.deepEqual(tails, [200, 0, 0]);
  assert.deepEqual(getRemoteAgentLog(AGENT).lines, ["tail"]);
  assert.deepEqual(errors, []);
});

test("an agent that never answers is reported", () => {
  mock.timers.enable({ apis: ["setInterval", "setTimeout"] });
  const errors = [];

  const stop = followRemoteAgentLog(
    AGENT,
    120,
    (error) => errors.push(error.message),
    async () => {},
  );
  mock.timers.tick(10_000);
  stop();

  assert.equal(errors.length, 1);
  assert.match(errors[0], /has not sent its log/);
});
