import type { AtCommandDoc, AtDocsFile, KnowledgeStore, ModuleProfile } from "./types";

export type DocumentApplicability = "exact" | "series" | "generic" | "unknown" | "incompatible";

function normalizeModels(models: string[] | undefined): string[] {
  return (models ?? []).map((model) => model.trim().toUpperCase()).filter(Boolean);
}

export function documentApplicability(
  profile: ModuleProfile | undefined,
  models: string[] | undefined,
  scopeSource?: string
): DocumentApplicability {
  if (scopeSource?.toLowerCase() === "unknown") return "unknown";
  const normalized = normalizeModels(models);
  if (!normalized.length) return "generic";
  if (!profile || profile.id.toLowerCase() === "generic" || profile.support_level === "generic") {
    return "incompatible";
  }

  const exact = new Set(normalizeModels(profile.model_codes));
  if (normalized.some((model) => exact.has(model))) return "exact";

  const series = profile.series.trim().toUpperCase();
  if (series && normalized.some((model) => model === series || model.startsWith(series))) {
    return "series";
  }
  return "incompatible";
}

function inheritedModels(file: AtDocsFile, command: AtCommandDoc): string[] {
  if (command.models?.length) return command.models;
  return Array.isArray(file.applies_to) ? file.applies_to : [];
}

function commandVariantCompatible(profile: ModuleProfile | undefined, command: AtCommandDoc): boolean {
  const variants = command.variants ?? [];
  const sources = [command.source, ...(command.sources ?? [])]
    .filter((source): source is string => Boolean(source));
  const nbOnly = variants.some((variant) => /^(?:nb-iot|nbiot)$/i.test(variant)) ||
    (sources.length > 0 && sources.every((source) =>
      /(?:^|[\\/])(?:nb_topics|common_basic_topics)(?:[\\/]|$)/i.test(source)
    ));
  if (!nbOnly) return true;
  return Boolean(profile && (
    profile.family.toLowerCase() === "nb-iot" ||
    profile.capabilities.some((capability) => capability.toLowerCase() === "nb-iot")
  ));
}

export interface ScopedAtCommand {
  command: AtCommandDoc;
  applicability: Exclude<DocumentApplicability, "incompatible" | "unknown">;
}

export function applicableAtCommands(
  knowledge: KnowledgeStore | null,
  profile: ModuleProfile | undefined
): ScopedAtCommand[] {
  return (knowledge?.at_docs ?? []).flatMap((file) =>
    file.commands.flatMap((command) => {
      if (!commandVariantCompatible(profile, command)) return [];
      const applicability = documentApplicability(
        profile,
        inheritedModels(file, command),
        command.scope_source
      );
      return applicability === "incompatible" || applicability === "unknown"
        ? []
        : [{ command, applicability }];
    })
  );
}
