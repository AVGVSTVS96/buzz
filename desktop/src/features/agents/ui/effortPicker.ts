import type {
  AcpConfigOptionValue,
  ManagedAgentBackend,
  RuntimeConfigSurface,
} from "@/shared/api/types";
import type { PersonaDropdownOption } from "./agentConfigOptions";
import { resolveModelCapabilities } from "./modelCapabilities";

/**
 * Sentinel dropdown value for "no explicit effort" — reverts the agent to the
 * adapter default at the next spawn. Distinct from any adapter option value.
 */
export const EFFORT_DEFAULT_DROPDOWN_VALUE = "__effort_default__";

/**
 * Pure gating + option compute for the effort write control in the edit dialog.
 *
 * The picker is a LOCAL-only, Save-gated write control: the dialog embeds the
 * selection in the locked `update_managed_agent` payload (PR #4625), which the
 * Rust backend rejects for non-local backends (remote effort is set at deploy
 * time via `policy_env`). So the UI must not offer it for a provider backend,
 * and there's nothing to pick until `effortChoices` knows the model's levels.
 *
 * `visible` is the single gate the dialog renders on: local backend AND known
 * choices.
 */
export function effortPickerState({
  backend,
  effortOptions,
  currentEffort,
}: {
  backend: ManagedAgentBackend;
  effortOptions: readonly AcpConfigOptionValue[] | undefined;
  currentEffort: string | null;
}): {
  visible: boolean;
  options: PersonaDropdownOption[];
  selectValue: string;
} {
  const visible = backend.type === "local" && effortOptions !== undefined;

  const options: PersonaDropdownOption[] = [
    { label: "Adapter default", value: EFFORT_DEFAULT_DROPDOWN_VALUE },
    ...(effortOptions ?? []).map((option) => ({
      label: option.displayName ?? option.value,
      value: option.value,
    })),
  ];

  // Preselect the currently-configured effort when it maps to a known option;
  // otherwise fall back to the adapter-default sentinel (also the null case).
  const trimmed = currentEffort?.trim() ?? "";
  const selectValue =
    trimmed.length > 0 &&
    (effortOptions ?? []).some((option) => option.value === trimmed)
      ? trimmed
      : EFFORT_DEFAULT_DROPDOWN_VALUE;

  return { visible, options, selectValue };
}

/**
 * Map a dropdown selection back to the persisted value sent as
 * `effortLevel` in the locked update payload: the sentinel clears effort
 * (null → adapter default), any other value is the explicit effort level.
 */
export function effortSelectionToPersistedValue(value: string): string | null {
  return value === EFFORT_DEFAULT_DROPDOWN_VALUE ? null : value;
}

/** Offered effort levels; `undefined` hides the picker. */
export type EffortOptions = readonly AcpConfigOptionValue[] | undefined;

/** Model ids in precedence order: explicit/persona, global, adapter default. */
export type EffortModels = readonly (string | null | undefined)[];

const CLAUDE_ALIAS_EFFORTS = ["low", "medium", "high"];

/**
 * Effort levels to offer for the model that will actually run, or `undefined`
 * to hide the picker. Claude levels always come from the capability manifest.
 * Other runtimes keep native-only behavior: the running session's own list
 * while the runtime is unchanged (`sessionApplies`). The first
 * non-blank of `models` wins. A blank id must never reach the manifest: its blank
 * fallback is adaptive and would invent levels for an unknown default.
 */
export function effortChoices({
  runtimeId,
  models,
  sessionApplies,
  session,
}: {
  runtimeId: string | undefined;
  models: EffortModels;
  sessionApplies: boolean;
  session?: RuntimeConfigSurface;
}): EffortOptions {
  // Claude's stored surface does not record which model its session ran, so
  // its levels cannot be trusted for the saved model; the manifest decides.
  if (
    runtimeId !== "claude" &&
    sessionApplies &&
    session?.effortConfigId !== undefined
  ) {
    return session.effortOptions ?? [];
  }
  const id = models.map((model) => model?.trim()).find(Boolean);
  if (runtimeId !== "claude" || !id) {
    return undefined;
  }
  const alias = id.toLowerCase().replace(/\[1m\]$/, "");
  const { thinkingMode, supportedEfforts } = resolveModelCapabilities(
    "anthropic",
    id,
  );
  const levels =
    alias === "opus" || alias === "sonnet"
      ? CLAUDE_ALIAS_EFFORTS
      : thinkingMode === "adaptive" || thinkingMode === "manual-budget"
        ? supportedEfforts
        : [];
  return levels.length > 0 ? levels.map((value) => ({ value })) : undefined;
}

/** Whether a pending effort selection may be saved for the given choices. */
export function isSavableEffort(
  level: string | null,
  choices: EffortOptions,
): boolean {
  return (
    level === null || (choices ?? []).some((choice) => choice.value === level)
  );
}
