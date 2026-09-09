import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("Kilo mode requires the bounded lifecycle and explicit proposal review", async () => {
  const mode = await readFile(new URL("./middleman-mode.yaml", import.meta.url), "utf8");
  for (const required of [
    "middleman index --changed",
    "middleman task start",
    "middleman_prepare",
    "middleman_expand",
    "middleman task finish",
    "middleman_propose",
    "never apply or\n      reject it automatically",
  ]) {
    assert.match(mode, new RegExp(required.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")));
  }
});
