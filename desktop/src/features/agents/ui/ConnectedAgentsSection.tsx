import * as React from "react";
import { EllipsisVertical, OctagonX, Unlink } from "lucide-react";
import { toast } from "sonner";

import {
  useDisconnectManagedAgentMutation,
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { isAgentDirectoryReady } from "@/features/agents/lib/agentAutocompleteEligibility";
import { requestAgentShutdown } from "@/features/agents/lib/managedAgentControlActions";
import { ownedAgentsRunningElsewhere } from "@/features/agents/lib/otherSetupAgent";
import { useAgentAvailabilityLookup } from "@/features/agents/lib/useAgentAvailability";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useUserProfileQuery } from "@/features/profile/hooks";
import {
  DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  ProfileAvatarWithStatus,
  scaleProfileAvatarStatusGeometry,
} from "@/features/profile/ui/ProfileAvatarWithStatus";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { PresenceStatus, RelayAgent } from "@/shared/api/types";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { OtherSetupAgentMarker } from "./OtherSetupAgentMarker";
import { IDENTITY_CARD_GRID_CLASS } from "./UnifiedAgentsSection";

const AVATAR_SIZE = 96;
const AVATAR_STATUS_GEOMETRY = scaleProfileAvatarStatusGeometry(
  DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  AVATAR_SIZE,
);

/** The viewer's agents that run somewhere this device does not manage. */
export function ConnectedAgentsSection({
  onOpenAgentProfile,
}: {
  onOpenAgentProfile: (pubkey: string) => void;
}) {
  const currentPubkey = useIdentityQuery().data?.pubkey;
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  const agents = ownedAgentsRunningElsewhere({
    currentPubkey,
    localInventoryReady:
      isAgentDirectoryReady(managedAgentsQuery) &&
      isAgentDirectoryReady(relayAgentsQuery),
    managedAgents: managedAgentsQuery.data ?? [],
    relayAgents: relayAgentsQuery.data ?? [],
  });
  const channelsQuery = useChannelsQuery({ enabled: agents.length > 0 });
  const { getAvailability } = useAgentAvailabilityLookup(
    agents.map((agent) => agent.pubkey),
  );
  const disconnectMutation = useDisconnectManagedAgentMutation();
  const [agentToDisconnect, setAgentToDisconnect] =
    React.useState<RelayAgent | null>(null);

  if (agents.length === 0) return null;

  async function handleShutdown(agent: RelayAgent) {
    try {
      const { noticeMessage } = await requestAgentShutdown(agent, {
        channels: channelsQuery.data ?? [],
        relayAgents: relayAgentsQuery.data ?? [],
      });
      if (noticeMessage) toast.success(noticeMessage);
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    }
  }

  async function handleDisconnect(agent: RelayAgent) {
    try {
      const syncError = await disconnectMutation.mutateAsync(agent.pubkey);
      if (syncError) {
        toast.warning(
          `Disconnected ${agent.name}. Buzz will retry publishing the change: ${syncError}`,
        );
      } else {
        toast.success(`Disconnected ${agent.name}.`);
      }
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <section className="space-y-2" data-testid="connected-agents">
      <h2 className="px-1 text-sm font-medium">
        Running elsewhere{" "}
        <span className="text-xs font-normal text-muted-foreground">
          ({agents.length})
        </span>
      </h2>
      <div className={IDENTITY_CARD_GRID_CLASS}>
        {agents.map((agent) => (
          <ConnectedAgentCard
            agent={agent}
            availability={getAvailability(agent.pubkey)}
            isPending={disconnectMutation.isPending}
            key={agent.pubkey}
            onDisconnect={setAgentToDisconnect}
            onOpenProfile={onOpenAgentProfile}
            onShutdown={(target) => void handleShutdown(target)}
          />
        ))}
      </div>
      <AlertDialog
        onOpenChange={(open) => {
          if (!open) setAgentToDisconnect(null);
        }}
        open={agentToDisconnect !== null}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Disconnect {agentToDisconnect?.name}?
            </AlertDialogTitle>
            <AlertDialogDescription>
              This removes it from your agents and archives its identity, so it
              no longer appears in member lists or mention suggestions. It
              cannot take back the tag you gave it: until its key is retired, it
              can still sign in as your agent. Shut it down first if it should
              stop. If Buzz on another of your computers runs it, delete it
              there instead.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button type="button" variant="outline">
                Cancel
              </Button>
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                onClick={() => {
                  if (agentToDisconnect) {
                    void handleDisconnect(agentToDisconnect);
                  }
                }}
                type="button"
                variant="destructive"
              >
                Disconnect
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}

function ConnectedAgentCard({
  agent,
  availability,
  isPending,
  onDisconnect,
  onOpenProfile,
  onShutdown,
}: {
  agent: RelayAgent;
  availability: PresenceStatus | undefined;
  isPending: boolean;
  onDisconnect: (agent: RelayAgent) => void;
  onOpenProfile: (pubkey: string) => void;
  onShutdown: (agent: RelayAgent) => void;
}) {
  const avatarUrl = useUserProfileQuery(agent.pubkey).data?.avatarUrl ?? null;

  return (
    <AgentIdentityCard
      actions={
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <button
              aria-label={`Open actions for ${agent.name}`}
              className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              type="button"
            >
              <EllipsisVertical className="h-4 w-4" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            onCloseAutoFocus={(event) => event.preventDefault()}
          >
            <DropdownMenuItem
              disabled={availability === "offline"}
              onClick={() => onShutdown(agent)}
            >
              <OctagonX className="h-4 w-4" />
              Shut down
            </DropdownMenuItem>
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              disabled={isPending}
              onClick={() => onDisconnect(agent)}
            >
              <Unlink className="h-4 w-4" />
              Disconnect
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      }
      ariaLabel={`${agent.name} agent profile`}
      avatar={
        <ProfileAvatarWithStatus
          avatarUrl={avatarUrl}
          className="h-24 w-24"
          geometry={AVATAR_STATUS_GEOMETRY}
          label={agent.name}
          shape="squircle"
          size={AVATAR_SIZE}
          status={availability}
        />
      }
      avatarUrl={avatarUrl}
      dataTestId={`connected-agent-${agent.pubkey}`}
      footerAccessory={<OtherSetupAgentMarker />}
      label={agent.name}
      onClick={() => onOpenProfile(agent.pubkey)}
    />
  );
}
