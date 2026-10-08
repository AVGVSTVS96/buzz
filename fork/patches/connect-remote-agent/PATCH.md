---
format: patch-md/v0.1
id: connect-remote-agent
summary: Owners connect an agent that runs elsewhere by its public key, see it under Running elsewhere, and can stop its turns.
baseline: a918e605cae84789d7ec47df152219b4c341a1c7
patch_file: connect-remote-agent.patch
patch_sha256: 7ca216aa424eb9e55bd3a7af48903db6aa891871e91127c7946dc1c1be0e156c
---

## Intent

**Connect agent** on the Agents page takes an agent's public key (npub or
hex), a name and who it answers, shows a review step, then signs the NIP-OA
auth tag for that key with the owner key and publishes the owner's
kind:30177 policy through the existing retention and flush loop. It hands
back the launcher env (`BUZZ_RELAY_URL`, `BUZZ_AUTH_TAG`,
`BUZZ_ACP_RESPOND_TO` matching the policy, `BUZZ_ACP_RELAY_OBSERVER=true`)
and a one-time `buzz users set-profile`. A **Running elsewhere** section lists
the viewer's agents this device holds no record for, with relay presence,
**Shut down** (owner `!shutdown`) and **Disconnect**. The activity pane's
Stop (owner-signed `cancel_turn`) shows for any relay agent whose verified
owner is the viewer. The kind:30177 builder and retain engine are split from
`ManagedAgentRecord` so a policy can be published for any agent key.

## Invariants

1. The agent's private key never reaches this device.
2. Connecting again keeps an existing policy and only issues a new tag.
3. Connect refuses the owner's own key, a key managed on this device, and a
   key whose profile carries another owner's tag.
4. Record-backed agents behave exactly as upstream.

## Verification

Desktop `cargo test --lib agent_connect`, `pnpm test`
(`connectedAgentHandoff`, `otherSetupAgent`, `unknownRelayAgentStatus`) and
`tsc --noEmit`. End to end: a buzz-acp agent launched with the handed-back
env shows as the owner's agent under Running elsewhere and answers.

## Removal

Remove when block/buzz closes #8012 with this PR or an equivalent.
