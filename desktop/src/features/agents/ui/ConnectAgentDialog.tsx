import * as React from "react";

import { useConnectManagedAgentMutation } from "@/features/agents/hooks";
import {
  connectedAgentEnv,
  connectedAgentProfileCommand,
  looksLikeSecretKey,
} from "@/features/agents/lib/connectedAgentHandoff";
import { useAgentAccessOwnerOnlyQuery } from "@/features/agents/useAgentAccessOwnerOnly";
import type { ConnectedAgentHandoff } from "@/shared/api/tauriManagedAgents";
import type { RespondToMode } from "@/shared/api/types";
import { parsePubkeyInput, pubkeyToNpub } from "@/shared/lib/nostrUtils";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { CopyButton } from "./CopyButton";
import {
  CreateAgentRespondToField,
  OWNER_ONLY_ACCESS_DISABLED_REASON,
} from "./RespondToField";

export function ConnectAgentDialog({
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-lg">
        {open ? <ConnectAgentFlow onDone={() => onOpenChange(false)} /> : null}
      </DialogContent>
    </Dialog>
  );
}

function ConnectAgentFlow({ onDone }: { onDone: () => void }) {
  const connectMutation = useConnectManagedAgentMutation();
  const accessLocked = useAgentAccessOwnerOnlyQuery().data === true;
  const [isReviewing, setIsReviewing] = React.useState(false);
  const [pubkeyInput, setPubkeyInput] = React.useState("");
  const [name, setName] = React.useState("");
  const [respondTo, setRespondTo] = React.useState<RespondToMode>("owner-only");
  const [respondToAllowlist, setRespondToAllowlist] = React.useState<string[]>(
    [],
  );

  const agentPubkey = parsePubkeyInput(pubkeyInput);
  const pubkeyError = looksLikeSecretKey(pubkeyInput)
    ? "That is a private key. Keep it where the agent runs and enter its public key instead."
    : pubkeyInput.trim() && !agentPubkey
      ? "Enter an npub or a 64-character hex public key."
      : null;
  const effectiveRespondTo = accessLocked ? "owner-only" : respondTo;
  const canContinue =
    agentPubkey !== null &&
    name.trim().length > 0 &&
    (effectiveRespondTo !== "allowlist" || respondToAllowlist.length > 0);

  if (connectMutation.data) {
    return (
      <ConnectAgentHandoff handoff={connectMutation.data} onDone={onDone} />
    );
  }

  if (isReviewing && agentPubkey) {
    return (
      <>
        <DialogHeader>
          <DialogTitle>Connect {name.trim()}?</DialogTitle>
          <DialogDescription>
            Buzz will sign, with your key, a statement that this key belongs to
            your agent.
          </DialogDescription>
        </DialogHeader>
        <p
          className="rounded-lg border border-border/70 bg-muted/50 px-3 py-2 font-mono text-xs break-all"
          data-testid="connect-agent-review-npub"
        >
          {pubkeyToNpub(agentPubkey)}
        </p>
        <ul className="list-disc space-y-2 pl-5 text-sm text-muted-foreground">
          <li>The agent&apos;s private key stays where it runs.</li>
          <li>
            Whoever holds that key can sign in here as your agent, even where
            only members are let in, and instruct your agents that answer only
            you.
          </li>
          <li>
            You can disconnect it later, but you cannot take the signature back:
            it stays valid until the agent&apos;s key is retired.
          </li>
          <li>
            Check the key with whoever runs the agent. A wrong key does nothing
            on its own.
          </li>
        </ul>
        {connectMutation.error instanceof Error ? (
          <p className="text-sm text-destructive">
            {connectMutation.error.message}
          </p>
        ) : null}
        <div className="flex justify-end gap-2">
          <Button
            disabled={connectMutation.isPending}
            onClick={() => setIsReviewing(false)}
            size="sm"
            type="button"
            variant="outline"
          >
            Back
          </Button>
          <Button
            disabled={connectMutation.isPending}
            onClick={() =>
              connectMutation.mutate({
                agentPubkey,
                name: name.trim(),
                respondTo: effectiveRespondTo,
                respondToAllowlist:
                  effectiveRespondTo === "allowlist" ? respondToAllowlist : [],
              })
            }
            size="sm"
            type="button"
          >
            {connectMutation.isPending ? "Connecting..." : "Sign and connect"}
          </Button>
        </div>
      </>
    );
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>Connect an agent</DialogTitle>
        <DialogDescription>
          Make an agent that runs somewhere else one of yours. Its private key
          never leaves the machine it runs on.
        </DialogDescription>
      </DialogHeader>
      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor="connect-agent-pubkey">
          Agent public key
        </label>
        <Input
          aria-describedby={
            pubkeyError ? "connect-agent-pubkey-error" : undefined
          }
          aria-invalid={pubkeyError ? true : undefined}
          id="connect-agent-pubkey"
          onChange={(event) => setPubkeyInput(event.target.value)}
          placeholder="npub1..."
          value={pubkeyInput}
        />
        {pubkeyError ? (
          <p
            className="text-sm text-destructive"
            id="connect-agent-pubkey-error"
          >
            {pubkeyError}
          </p>
        ) : null}
      </div>
      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor="connect-agent-name">
          Name
        </label>
        <Input
          id="connect-agent-name"
          onChange={(event) => setName(event.target.value)}
          value={name}
        />
      </div>
      <CreateAgentRespondToField
        allowlist={accessLocked ? [] : respondToAllowlist}
        disabled={accessLocked}
        disabledReason={
          accessLocked ? OWNER_ONLY_ACCESS_DISABLED_REASON : undefined
        }
        mode={effectiveRespondTo}
        onAllowlistChange={setRespondToAllowlist}
        onModeChange={setRespondTo}
        runLocation="remote"
      />
      <div className="flex justify-end gap-2">
        <Button onClick={onDone} size="sm" type="button" variant="outline">
          Cancel
        </Button>
        <Button
          disabled={!canContinue}
          onClick={() => {
            connectMutation.reset();
            setIsReviewing(true);
          }}
          size="sm"
          type="button"
        >
          Continue
        </Button>
      </div>
    </>
  );
}

function ConnectAgentHandoff({
  handoff,
  onDone,
}: {
  handoff: ConnectedAgentHandoff;
  onDone: () => void;
}) {
  const env = connectedAgentEnv(handoff);
  const profileCommand = connectedAgentProfileCommand(handoff);

  return (
    <>
      <DialogHeader>
        <DialogTitle>Hand the tag to {handoff.policy.name}</DialogTitle>
        <DialogDescription>
          Add these lines to the agent&apos;s environment, next to its own
          BUZZ_PRIVATE_KEY, and restart it.
        </DialogDescription>
      </DialogHeader>
      {handoff.policyKept ? (
        <p className="text-sm text-muted-foreground">
          {handoff.policy.name} was already one of your agents, so its settings
          were kept.
        </p>
      ) : null}
      <HandoffBlock label="Copy environment" testId="connect-agent-env">
        {env}
      </HandoffBlock>
      <p className="text-sm text-muted-foreground">
        Then publish its profile once from the same environment, so Buzz shows
        it as yours:
      </p>
      <HandoffBlock label="Copy command" testId="connect-agent-profile-command">
        {profileCommand}
      </HandoffBlock>
      <p className="text-sm text-muted-foreground">
        It then appears under Running elsewhere. Mention it in a channel to
        invite it there.
      </p>
      {handoff.policySyncError ? (
        <p className="text-sm text-warning">
          Its settings are saved but not published yet (
          {handoff.policySyncError}). Buzz will retry.
        </p>
      ) : null}
      <div className="flex justify-end">
        <Button onClick={onDone} size="sm" type="button">
          Done
        </Button>
      </div>
    </>
  );
}

function HandoffBlock({
  children,
  label,
  testId,
}: {
  children: string;
  label: string;
  testId: string;
}) {
  return (
    <div className="space-y-2">
      <pre
        className="max-h-48 overflow-auto rounded-lg border border-border/70 bg-muted/50 px-3 py-2 font-mono text-xs whitespace-pre-wrap break-all"
        data-testid={testId}
      >
        {children}
      </pre>
      <div className="flex justify-end">
        <CopyButton label={label} value={children} />
      </div>
    </div>
  );
}
