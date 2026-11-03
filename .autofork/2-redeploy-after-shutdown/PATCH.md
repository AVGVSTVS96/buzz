An owner can deploy a provider agent again after shutting it down. An owned provider agent keeps `status=deployed` after `!shutdown`, so its profile only offered Shutdown and there was no way back (block/buzz#7724).

Its profile gets an explicit **Deploy again** action that repeats the existing same-identity provider handoff; the provider's own idempotency decides whether a live instance is reused or replaced. Deploy again keeps the agent's identity (the same pubkey) and never stops, deletes or creates an agent, and a deploy that fails can be retried from the same profile. Shutdown and relay presence stay independent of each other, and the toast says a deployment was requested rather than that a process restarted.

`desktop/tests/e2e/agent-availability.spec.ts` (the integration project, with the mock bridge) covers it, including provider failure, retry and the same pubkey, along with `tsc --noEmit` and `pnpm test`.

This carries @FabianHertwig's block/buzz#7485 unchanged. Once block/buzz merges #7485, or another fix for #7724 that offers the same action, this patch is done.
