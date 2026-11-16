import assert from "node:assert/strict";
import { test } from "node:test";
import { MAX_BYTES, MAX_FILES, accept, named } from "./attachments.ts";

const file = (name: string, size = 1, lastModified = 1) =>
  new File([new Uint8Array(size)], name, { type: "text/plain", lastModified });

test("files are added within the limits, and the rest are explained", () => {
  const { files, refused } = accept([file("a.md")], [file(".env"), file("big.bin", MAX_BYTES + 1), file("b.csv")]);
  assert.deepEqual(
    files.map((f) => f.name),
    ["a.md", "b.csv"],
  );
  assert.equal(refused, ".env can't be uploaded because its name starts with a dot. big.bin is over the 20 MB limit.");
  const single = accept([], [file("ok.txt")]);
  assert.equal(single.files.length, 1);
  assert.equal(single.refused, null);
});

test("a message holds at most ten files, and the same file is attached once", () => {
  const many = Array.from({ length: MAX_FILES + 2 }, (_, i) => file(`f${i}.txt`));
  const { files, refused } = accept([], many);
  assert.equal(files.length, MAX_FILES);
  assert.equal(refused, `A message can carry at most ${MAX_FILES} files.`);
  const same = file("same.txt", 3, 7);
  assert.equal(accept([same], [file("same.txt", 3, 7)]).files.length, 1);
  assert.equal(accept([same], [file("same.txt", 3, 8)]).files.length, 2);
});

test("a pasted picture is named for when it was pasted; other files keep their names", () => {
  const pasted = new File([new Uint8Array(4)], "image.png", { type: "image/png" });
  const renamed = named(pasted, new Date(2026, 10, 14, 9, 5, 7));
  assert.equal(renamed.name, "Pasted 2026-11-14 09.05.07.png");
  assert.equal(renamed.type, "image/png");
  assert.equal(renamed.size, 4);
  const kept = file("photo.png");
  assert.equal(named(kept, new Date()), kept);
});
