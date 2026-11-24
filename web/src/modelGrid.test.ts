import assert from "node:assert/strict";
import { test } from "node:test";
import type { Model } from "./api.ts";
import { columns, families, label, providerName, providers } from "./modelGrid.ts";

const FIVE = ["low", "medium", "high", "xhigh", "max"];

function model(id: string, provider: string, family: string, version: string, efforts: string[]): Model {
  return { id, provider, family, version, efforts, effort: efforts[0] ?? "", note: "" };
}

// The catalog's own order: Sol's versions are neither together nor newest first.
const MODELS = [
  model("gpt-6-luna", "openai", "Luna", "6", ["none", ...FIVE]),
  model("gpt-6.1-terra", "openai", "Terra", "6.1", []),
  model("gpt-6.1-sol", "openai", "Sol", "6.1", FIVE),
  model("gpt-6-sol", "openai", "Sol", "6", ["none", ...FIVE]),
  model("claude-opus-5-5", "anthropic", "Opus", "5.5", FIVE),
  model("gpt-6.2-sol", "openai", "Sol", "6.2", FIVE),
];

test("each provider is a tab, named the way people write it", () => {
  assert.deepEqual(providers(MODELS), ["openai", "anthropic"]);
  assert.deepEqual(["openai", "anthropic", "gemini"].map(providerName), ["OpenAI", "Anthropic", "Gemini"]);
});

test("a family is one row whose versions run newest first", () => {
  const rows = families(MODELS, "openai");
  assert.deepEqual(rows.map((r) => r.name), ["Luna", "Terra", "Sol"]);
  assert.deepEqual(rows[2].versions.map((m) => m.id), ["gpt-6.2-sol", "gpt-6.1-sol", "gpt-6-sol"]);
  assert.deepEqual(families(MODELS, "anthropic").map((r) => r.name), ["Opus"]);
});

test("versions compare number by number", () => {
  const rows = families(
    [model("a", "p", "F", "6.2", []), model("b", "p", "F", "6.10", []), model("c", "p", "F", "7", [])],
    "p",
  );
  assert.deepEqual(rows[0].versions.map((m) => m.version), ["7", "6.10", "6.2"]);
});

test("effort columns merge every model's scale, lowest first", () => {
  assert.deepEqual(columns(MODELS), ["none", ...FIVE]);
  const narrowFirst = [model("a", "p", "A", "1", ["low", "high"]), model("b", "p", "B", "1", ["none", "low", "medium", "high", "max"])];
  assert.deepEqual(columns(narrowFirst), ["none", "low", "medium", "high", "max"]);
  assert.deepEqual(columns([MODELS[1]]), []);
});

test("the button names the family, version and effort", () => {
  assert.equal(label(MODELS, { model: "gpt-6.2-sol", effort: "high" }), "Sol 6.2 · high");
  assert.equal(label(MODELS, { model: "gpt-6.1-terra", effort: "" }), "Terra 6.1");
  assert.equal(label(MODELS, { model: "gpt-6.1-sol", effort: null }), "Sol 6.1");
  assert.equal(label(MODELS, { model: "retired-model", effort: "low" }), "retired-model · low");
  assert.equal(label(MODELS, null), "Model");
});
