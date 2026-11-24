import type { Model } from "./api.ts";

/** Choice is a model and the reasoning effort it runs at; the effort is null where it is not
 *  known, and empty for a model that takes none. */
export type Choice = { model: string; effort: string | null };

/** Family is one model line as a row of the picker, with its newest version first. */
export type Family = { name: string; versions: Model[] };

/** Providers lists the providers of the offered models, in the catalog's order. */
export function providers(models: Model[]): string[] {
  return [...new Set(models.map((m) => m.provider))];
}

/** ProviderName writes a provider's ID the way people write the name. */
export function providerName(provider: string): string {
  const names: Record<string, string> = { openai: "OpenAI", anthropic: "Anthropic" };
  return names[provider] ?? provider.charAt(0).toUpperCase() + provider.slice(1);
}

/** Families groups a provider's models into rows, in the catalog's order. */
export function families(models: Model[], provider: string): Family[] {
  const rows: Family[] = [];
  for (const model of models.filter((m) => m.provider === provider)) {
    const row = rows.find((r) => r.name === model.family);
    if (row) row.versions.push(model);
    else rows.push({ name: model.family, versions: [model] });
  }
  for (const row of rows) row.versions.sort((a, b) => compare(b.version, a.version));
  return rows;
}

/** Compare orders dotted versions number by number, so 6.10 comes after 6.2. */
function compare(a: string, b: string): number {
  const [x, y] = [a, b].map((version) => version.split(".").map(Number));
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const difference = (x[i] ?? 0) - (y[i] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

/** Columns merges the effort scales of several models into one, lowest first. An effort only
 *  some models take lands where those models have it. */
export function columns(models: Model[]): string[] {
  const merged: string[] = [];
  for (const model of models) {
    let at = 0;
    for (const effort of model.efforts) {
      const found = merged.indexOf(effort);
      if (found < 0) merged.splice(at, 0, effort);
      at = (found < 0 ? at : found) + 1;
    }
  }
  return merged;
}

/** Label names a choice the way the picker's button shows it: "Sol 6.2 · high". */
export function label(models: Model[], choice: Choice | null): string {
  if (!choice) return "Model";
  const model = models.find((m) => m.id === choice.model);
  const name = model ? `${model.family} ${model.version}` : choice.model;
  return choice.effort ? `${name} · ${choice.effort}` : name;
}
