Agent cards in Pulse show whether the agent is really online. Pulse read the roster's `status` field, so a deployed agent looked online whether or not it was running.

In `desktop/src/features/pulse/ui/PulseView.tsx`, each card's status dot comes from `useAgentAvailabilityLookup(agentPubkeys).getAvailability(pubkey)`, the same availability reader the Agents page and profiles use, instead of the roster's `status`. Roster entries built from local managed records carry `status: "unknown"` rather than mapping `running` or `deployed` to online. Pulse never derives availability from a managed record's lifecycle status or from kind:10100 `status`, only from relay presence through the shared reader, and when a presence read fails or the relay is disconnected, the card shows no dot (unknown).

The test "Pulse shows relay presence for an agent, not its saved deployment" in `desktop/tests/e2e/agent-availability.spec.ts` (integration project, mock bridge) checks that a deployed provider agent shows "Agent offline", then "Agent online" once presence arrives, along with `tsc --noEmit` and `pnpm test`.

Once block/buzz's main reads Pulse agent status through `useAgentAvailabilityLookup` (the upstream PR drafted from the branch `fix/pulse-agent-presence`, or another fix for #4537), this patch is done.
