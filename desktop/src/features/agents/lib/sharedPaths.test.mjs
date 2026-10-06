import assert from "node:assert/strict";
import test from "node:test";

import {
  parseSharedPaths,
  sharedPathsError,
  sharedPathsUpdate,
} from "./sharedPaths.ts";

test("parseSharedPaths reads one trimmed path per line and skips blanks", () => {
  assert.deepEqual(parseSharedPaths(" PLANS \r\n\n notes/today.md\n  \n"), [
    "PLANS",
    "notes/today.md",
  ]);
  assert.deepEqual(parseSharedPaths(""), []);
});

test("sharedPathsError rejects a path the harness can't receive", () => {
  assert.equal(sharedPathsError("PLANS\nnotes/today.md"), null);
  assert.equal(
    sharedPathsError("PLANS\na,b.md"),
    "Paths can't contain commas.",
  );
});

test("sharedPathsUpdate omits an unchanged list", () => {
  assert.equal(
    sharedPathsUpdate("PLANS\n notes.md \n", ["PLANS", "notes.md"]),
    undefined,
  );
  assert.equal(sharedPathsUpdate("", []), undefined);
});

test("sharedPathsUpdate sends edits, reorders, and clears", () => {
  assert.deepEqual(sharedPathsUpdate("PLANS\nnew.md", ["PLANS"]), [
    "PLANS",
    "new.md",
  ]);
  assert.deepEqual(sharedPathsUpdate("b\na", ["a", "b"]), ["b", "a"]);
  assert.deepEqual(sharedPathsUpdate("  \n", ["PLANS"]), []);
});
