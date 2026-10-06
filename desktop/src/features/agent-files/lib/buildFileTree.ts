import type { AgentFileEntry } from "@/shared/api/tauriAgentFiles";

export type FileTreeNode =
  | { kind: "folder"; name: string; path: string; children: FileTreeNode[] }
  | { kind: "file"; name: string; path: string; file: AgentFileEntry };

/**
 * Turn the flat, `/`-separated paths an agent shares into a tree: folders
 * first, then files, each sorted by name. Whatever layout the agent shares
 * renders the same way.
 */
export function buildFileTree(files: AgentFileEntry[]): FileTreeNode[] {
  const root: FileTreeNode[] = [];
  const folders = new Map<string, FileTreeNode[]>([["", root]]);

  for (const file of files) {
    const segments = file.path.split("/");
    let parent = root;
    for (let depth = 0; depth < segments.length - 1; depth += 1) {
      const path = segments.slice(0, depth + 1).join("/");
      let children = folders.get(path);
      if (!children) {
        children = [];
        folders.set(path, children);
        parent.push({ kind: "folder", name: segments[depth], path, children });
      }
      parent = children;
    }
    parent.push({
      kind: "file",
      name: segments[segments.length - 1],
      path: file.path,
      file,
    });
  }

  for (const children of folders.values()) {
    children.sort((left, right) =>
      left.kind === right.kind
        ? left.name.localeCompare(right.name)
        : left.kind === "folder"
          ? -1
          : 1,
    );
  }
  return root;
}
