---
format: patch-md/v0.1
id: redeploy-after-shutdown
summary: Owners can deploy a remote agent again after shutting it down.
baseline: af5bb0af488784c3707e99c87e0675a729711a6e
patch_file: redeploy-after-shutdown.patch
patch_sha256: c927413a300e686ea17c2052e3babed24ea47f0432c12ae6142e40a1a8385c15
---

## Intent

An owned provider agent keeps `status=deployed` after `!shutdown`, so its
profile only offered Shutdown and there was no way back (#7724). Its profile
gets an explicit **Deploy again** action that repeats the existing
same-identity provider handoff; provider idempotency decides whether a live
instance is reused or replaced. Shutdown and relay presence stay independent,
and the toast says a deployment was requested rather than that a process
restarted. Carries @FabianHertwig's #7485 unchanged.

## Invariants

1. Deploy again keeps the agent's identity (same pubkey) and never stops,
   deletes or creates an agent.
2. A failed deploy can be retried from the same profile.

## Verification

`desktop/tests/e2e/agent-availability.spec.ts` (integration project, mock
bridge), including provider failure, retry and the same pubkey, plus
`tsc --noEmit` and `pnpm test`.

## Removal

Remove when block/buzz merges #7485, or another fix for #7724 that offers
the same action.
