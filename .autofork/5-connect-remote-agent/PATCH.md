An owner can connect an agent that runs somewhere else from Buzz Desktop, by its public key, without the agent's private key ever reaching this device (block/buzz#8012).

**Connect agent** on the Agents page takes an agent's public key (npub or hex), a name and who it answers, and shows a review step. Then it signs the NIP-OA auth tag for that key with the owner key and publishes the owner's kind:30177 policy through the existing retention and flush loop. It hands back the launcher env (`BUZZ_RELAY_URL`, `BUZZ_AUTH_TAG`, a `BUZZ_ACP_RESPOND_TO` matching the policy, and `BUZZ_ACP_RELAY_OBSERVER=true`) and a one-time `buzz users set-profile`. The kind:30177 builder and retain engine are split from `ManagedAgentRecord`, so a policy can be published for any agent key.

A **Running elsewhere** section lists the viewer's agents this device holds no record for, with their relay presence, **Shut down** (the owner's `!shutdown`) and **Disconnect**. The activity pane's Stop (an owner-signed `cancel_turn`) shows for any relay agent whose verified owner is the viewer.

What has to stay true: the agent's private key never reaches this device. Connecting again keeps an existing policy and only issues a new tag. Connect refuses the owner's own key, a key managed on this device, and a key whose profile carries another owner's tag. Agents backed by a record on this device behave exactly as upstream.

Desktop's `cargo test --lib agent_connect`, `pnpm test` (`connectedAgentHandoff`, `otherSetupAgent`, `unknownRelayAgentStatus`) and `tsc --noEmit` check it. End to end: a buzz-acp agent launched with the handed-back env shows as the owner's agent under Running elsewhere and answers.

Once block/buzz closes #8012 with this or an equivalent, this patch is done.
