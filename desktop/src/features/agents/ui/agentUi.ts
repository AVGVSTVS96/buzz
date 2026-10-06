import type { ManagedAgent } from "@/shared/api/types";

function commandLooksLikePath(command: string) {
  const trimmed = command.trim();
  return (
    trimmed.startsWith(".") ||
    trimmed.startsWith("~") ||
    trimmed.includes("/") ||
    trimmed.includes("\\")
  );
}

export function describeResolvedCommand(command: string, resolvedPath: string) {
  const normalized = resolvedPath.replace(/\\/g, "/");

  if (normalized.includes("/target/release/")) {
    return "workspace release build";
  }
  if (normalized.includes("/target/debug/")) {
    return "workspace debug build";
  }

  if (commandLooksLikePath(command)) {
    return "custom command";
  }

  return "installed on PATH";
}

export function describeAgentLog(
  agent: Pick<ManagedAgent, "backend" | "logPath">,
) {
  if (agent.backend.type !== "local") {
    return "streamed from the agent";
  }
  const normalized = agent.logPath.replace(/\\/g, "/");
  const basename = normalized.split("/").pop() ?? agent.logPath;

  if (!basename.endsWith(".log")) {
    return "local harness log";
  }

  const stem = basename.slice(0, -4);
  if (stem.length <= 18) {
    return basename;
  }

  return `${stem.slice(0, 8)}…${stem.slice(-6)}.log`;
}
