import { test } from "node:test";
import assert from "node:assert/strict";
import { validTitle } from "./check-pr-title.mjs";

test("conventional titles accept types, scopes and breaking changes", () => {
  for (const title of [
    "feat: add controls",
    "fix(vision): reject unknown screens",
    "refactor(core)!: replace the runner",
    "chore!: retire Python",
    "ci(deps): bump actions/checkout to v6",
  ])
    assert.ok(validTitle(title), title);
});
test("conventional titles reject missing fields, unknown types and extra lines", () => {
  for (const title of [
    "",
    "update stuff",
    "Feat: add controls",
    "misc: update",
    "fix: ",
    "fix:  ",
    "fix:missing space",
    "feat(): add controls",
    "fix: valid\nextra",
    "fix: valid\n",
    "fix: valid\rextra",
    undefined,
  ])
    assert.equal(validTitle(title), false, title);
});
