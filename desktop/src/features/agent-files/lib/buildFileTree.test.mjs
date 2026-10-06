import assert from "node:assert/strict";
import test from "node:test";

import { buildFileTree } from "./buildFileTree.ts";

function file(path) {
  return {
    path,
    sha256: `sha-${path}`,
    size: 1,
    content: "x",
    eventId: `id-${path}`,
    createdAt: 0,
  };
}

function shape(nodes) {
  return nodes.map((node) =>
    node.kind === "folder" ? { [node.name]: shape(node.children) } : node.name,
  );
}

test("buildFileTree: empty listing → empty tree", () => {
  assert.deepEqual(buildFileTree([]), []);
});

test("buildFileTree: nests paths into folders, folders before files", () => {
  const tree = buildFileTree([
    file("notes.md"),
    file("PLANS/b.md"),
    file("PLANS/archive/old.md"),
    file("AGENTS.md"),
    file("PLANS/a.md"),
  ]);
  assert.deepEqual(shape(tree), [
    { PLANS: [{ archive: ["old.md"] }, "a.md", "b.md"] },
    "AGENTS.md",
    "notes.md",
  ]);
});

test("buildFileTree: folder and file nodes carry full paths", () => {
  const [folder] = buildFileTree([file("a/b/c.txt")]);
  assert.equal(folder.path, "a");
  assert.equal(folder.children[0].path, "a/b");
  const leaf = folder.children[0].children[0];
  assert.equal(leaf.path, "a/b/c.txt");
  assert.equal(leaf.file.eventId, "id-a/b/c.txt");
});
