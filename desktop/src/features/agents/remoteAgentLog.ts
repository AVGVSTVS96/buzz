import * as React from "react";

import { followManagedAgentLog } from "@/shared/api/agentControl";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ObserverEvent } from "./ui/agentSessionTypes";

const AGENT_LOG_EVENT_KIND = "log";
/** Matches the harness's in-memory tail, the most an owner can request. */
const MAX_REMOTE_LOG_LINES = 1_000;
/** The harness streams for 60s after each request; renew well inside that. */
const LOG_FOLLOW_RENEW_MS = 30_000;
const LOG_FOLLOW_ANSWER_TIMEOUT_MS = 10_000;

type RemoteAgentLog = {
  answered: boolean;
  lines: readonly string[];
};

const UNANSWERED: RemoteAgentLog = { answered: false, lines: [] };

const logsByAgent = new Map<string, RemoteAgentLog>();
const listeners = new Set<() => void>();

function notify() {
  for (const listener of listeners) {
    listener();
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function isAgentLogEvent(event: ObserverEvent) {
  return event.kind === AGENT_LOG_EVENT_KIND;
}

export function getRemoteAgentLog(agentPubkey: string): RemoteAgentLog {
  return logsByAgent.get(normalizePubkey(agentPubkey)) ?? UNANSWERED;
}

/** Append the lines of a `log` observer event, marking any gap the harness reports. */
export function appendRemoteAgentLog(agentPubkey: string, payload: unknown) {
  const { dropped, lines } = (payload ?? {}) as {
    dropped?: unknown;
    lines?: unknown;
  };
  if (!Array.isArray(lines)) {
    return;
  }
  const gap =
    typeof dropped === "number" && dropped > 0
      ? [`[${dropped} log lines skipped]`]
      : [];
  const key = normalizePubkey(agentPubkey);
  logsByAgent.set(key, {
    answered: true,
    lines: [
      ...(logsByAgent.get(key)?.lines ?? []),
      ...gap,
      ...lines.filter((line): line is string => typeof line === "string"),
    ].slice(-MAX_REMOTE_LOG_LINES),
  });
  notify();
}

export function clearRemoteAgentLog(agentPubkey: string) {
  logsByAgent.delete(normalizePubkey(agentPubkey));
  notify();
}

export function resetRemoteAgentLogs() {
  logsByAgent.clear();
  notify();
}

/**
 * Ask the agent for a fresh tail of `lineCount` lines and keep its stream open
 * until the returned cleanup runs. `onError` reports a failed request, or an
 * agent that never answers (offline, or a harness without remote logs).
 */
export function followRemoteAgentLog(
  agentPubkey: string,
  lineCount: number,
  onError: (error: Error) => void,
  follow = followManagedAgentLog,
) {
  clearRemoteAgentLog(agentPubkey);
  const request = (tail: number) =>
    follow(agentPubkey, tail).catch((error) =>
      onError(error instanceof Error ? error : new Error(String(error))),
    );
  void request(lineCount);
  const renew = setInterval(() => void request(0), LOG_FOLLOW_RENEW_MS);
  const overdue = setTimeout(() => {
    if (!getRemoteAgentLog(agentPubkey).answered) {
      onError(
        new Error(
          "The agent has not sent its log. It may be offline, or running a harness without remote logs.",
        ),
      );
    }
  }, LOG_FOLLOW_ANSWER_TIMEOUT_MS);
  return () => {
    clearInterval(renew);
    clearTimeout(overdue);
  };
}

export function useRemoteAgentLog(
  agentPubkey: string | null,
  lineCount: number,
) {
  const log = React.useSyncExternalStore(subscribe, () =>
    agentPubkey ? getRemoteAgentLog(agentPubkey) : UNANSWERED,
  );
  const [error, setError] = React.useState<Error | null>(null);

  React.useEffect(() => {
    if (!agentPubkey) {
      return;
    }
    setError(null);
    return followRemoteAgentLog(agentPubkey, lineCount, setError);
  }, [agentPubkey, lineCount]);

  const content = React.useMemo(
    () => (log.answered ? log.lines.join("\n") : null),
    [log],
  );
  const unanswered = agentPubkey !== null && !log.answered;
  return {
    content,
    error: unanswered ? error : null,
    isLoading: unanswered && error === null,
  };
}
