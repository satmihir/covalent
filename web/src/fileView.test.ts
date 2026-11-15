import assert from "node:assert/strict";
import { test } from "node:test";
import type { Action } from "./api.ts";
import { LARGEST_TEXT, decode, kind, size, touched } from "./fileView.ts";

function action(tool: string, target: string | null, state: Action["state"] = "done"): Action {
  return { call_id: `${tool}-${target}-${state}`, tool, target, state, operation: null, error: null };
}

test("pictures show inline, small files as text, and the rest as downloads", () => {
  assert.equal(kind("boat.PNG", 10 * LARGEST_TEXT), "picture");
  assert.equal(kind("art/sketch.webp", 1), "picture");
  assert.equal(kind("notes.md", LARGEST_TEXT), "text");
  assert.equal(kind("Makefile", 10), "text");
  assert.equal(kind("png", 10), "text");
  assert.equal(kind("page.svg", 10), "text");
  assert.equal(kind("data.csv", LARGEST_TEXT + 1), "download");
});

test("only valid UTF-8 without NUL bytes counts as text", () => {
  const bytes = (values: number[]) => new Uint8Array(values).buffer;
  assert.equal(decode(new TextEncoder().encode("café\n").buffer as ArrayBuffer), "café\n");
  assert.equal(decode(bytes([0xff, 0xfe, 0x41])), null);
  assert.equal(decode(bytes([0x41, 0x00, 0x42])), null);
  assert.equal(decode(bytes([])), "");
});

test("sizes read the way a file list writes them", () => {
  assert.equal(size(0), "0 B");
  assert.equal(size(1023), "1023 B");
  assert.equal(size(1024), "1.0 KB");
  assert.equal(size(2560), "2.5 KB");
  assert.equal(size(50 * 1024), "50 KB");
  assert.equal(size(3 * 1024 * 1024), "3.0 MB");
});

test("a reply's touched files are the ones it finished writing, editing or drawing, once each", () => {
  const activity = [
    action("read_file", "a.md"),
    action("write_file", "b.md"),
    action("edit_file", "b.md"),
    action("edit_file", "c.md", "failed"),
    action("generate_image", "boat.png"),
    action("write_file", "d.md", "working"),
    action("delete_file", "e.md"),
    action("write_file", null),
  ];
  assert.deepEqual(touched(activity), ["b.md", "boat.png"]);
});
