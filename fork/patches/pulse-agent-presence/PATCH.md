---
format: patch-md/v0.1
id: pulse-agent-presence
summary: Pulse agent cards show the agent's real relay presence instead of offline for every relay agent and online forever for any deployed one.
baseline: af5bb0af488784c3707e99c87e0675a729711a6e
patch_file: pulse-agent-presence.patch
patch_sha256: 90ba3047bfd50dbb54b529d79cbb5843e793012f3d7bce35146e8bf1f18b2e48
---

## Intent

In `desktop/src/features/pulse/ui/PulseView.tsx`, the agent cards' status dot
comes from `useAgentAvailabilityLookup(agentPubkeys).getAvailability(pubkey)`,
the same availability reader the Agents page and profiles use, instead of the
roster's `status` field. Roster entries synthesized from local managed records
carry `status: "unknown"` rather than mapping `running`/`deployed` to online.

## Invariants

1. Pulse never derives availability from a managed record's lifecycle status
   or from kind:10100 `status`; only relay presence via the shared reader.
2. A failed presence read or disconnected relay shows no dot (unknown).

## Verification

`desktop/tests/e2e/agent-availability.spec.ts` test "Pulse shows relay
presence for an agent, not its saved deployment" (integration project, mock
bridge): a deployed provider agent shows "Agent offline", then "Agent online"
after presence arrives. Plus `tsc --noEmit` and `pnpm test`.

## Removal

Delete when upstream `block/buzz` main reads Pulse agent status through
`useAgentAvailabilityLookup` (the upstream PR drafted from branch
`fix/pulse-agent-presence`, or an equivalent fix for #4537, is merged).
