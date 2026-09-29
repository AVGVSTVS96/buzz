import type { InheritedDefault } from "./bakedEnvHelpers";

/** Match Goose launch precedence without persisting inherited values. */
export function resolveGooseConfig({
  env = {},
  provider,
  model,
  file,
  defaults = {},
}: {
  env?: Readonly<Record<string, string>>;
  provider?: string | null;
  model?: string | null;
  file?: { provider?: string | null; model?: string | null } | null;
  defaults?: Readonly<Record<string, string>>;
}): { provider: InheritedDefault; model: InheritedDefault } {
  const resolve = (
    key: string,
    structured?: string | null,
    fromFile?: string | null,
    useBuildDefault = true,
  ): InheritedDefault => {
    const candidates: InheritedDefault[] = [
      { value: env[key]?.trim() ?? "", source: "environment" },
      { value: structured?.trim() ?? "", source: "global" },
      { value: fromFile?.trim() ?? "", source: "file" },
      {
        value: useBuildDefault ? (defaults[key]?.trim() ?? "") : "",
        source: "build",
      },
    ];
    return (
      candidates.find(({ value }) => value.length > 0) ?? {
        value: "",
        source: null,
      }
    );
  };
  const resolvedProvider = resolve("GOOSE_PROVIDER", provider, file?.provider);
  return {
    provider: resolvedProvider,
    model: resolve(
      "GOOSE_MODEL",
      model,
      file?.model,
      !!resolvedProvider.value &&
        resolvedProvider.value === defaults.GOOSE_PROVIDER?.trim(),
    ),
  };
}
