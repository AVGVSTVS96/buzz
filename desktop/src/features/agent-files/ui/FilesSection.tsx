import * as React from "react";
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  FileText,
  Folder,
  FolderOpen,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { toast } from "sonner";

import {
  useAgentFileTree,
  useProposeAgentFileEdit,
} from "@/features/agent-files/hooks";
import type { FileTreeNode } from "@/features/agent-files/lib/buildFileTree";
import {
  formatFileSize,
  relativeTime,
} from "@/features/projects/lib/projectsViewHelpers";
import { languageForPath } from "@/features/projects/ui/ProjectRepositoryPanel";
import type {
  AgentFileEdit,
  AgentFileEntry,
} from "@/shared/api/tauriAgentFiles";
import { cn } from "@/shared/lib/cn";
import { Button, type ButtonProps } from "@/shared/ui/button";
import { Markdown, SyntaxHighlightedCode } from "@/shared/ui/markdown";
import { Skeleton } from "@/shared/ui/skeleton";
import { Textarea } from "@/shared/ui/textarea";

/** NIP-44 plaintext limit: larger files are listed without their text. */
const INLINE_LIMIT_BYTES = 65_535;

/**
 * Files section — the files an agent shares with its owner (NIP-AF), shown
 * as a tree whatever layout the agent uses. Owners can read each file and
 * propose an edit; the agent decides whether to apply it.
 *
 * Owner-gated by the caller exactly like {@link MemorySection}.
 */
export function FilesSection({
  agentPubkey,
  viewerIsOwner,
}: {
  agentPubkey: string;
  viewerIsOwner: boolean;
}): React.ReactElement | null {
  if (!viewerIsOwner) return null;

  return <FilesSectionForOwner agentPubkey={agentPubkey} />;
}

export function FilesRefreshButton({
  agentPubkey,
  viewerIsOwner,
  className,
  iconClassName,
  variant = "ghost",
}: {
  agentPubkey: string;
  viewerIsOwner: boolean;
  className?: string;
  iconClassName?: string;
  variant?: ButtonProps["variant"];
}): React.ReactElement | null {
  const { query } = useAgentFileTree(agentPubkey, { enabled: viewerIsOwner });

  if (!viewerIsOwner || !query.data) return null;

  return (
    <Button
      aria-label="Refresh files"
      className={cn(className, query.isFetching && "cursor-wait")}
      data-testid="agent-files-refetch"
      disabled={query.isFetching}
      onClick={() => query.refetch()}
      size="icon"
      type="button"
      variant={variant}
    >
      <RefreshCw
        className={cn(
          iconClassName ?? "h-4 w-4",
          query.isFetching && "animate-spin",
        )}
      />
    </Button>
  );
}

function FilesSectionForOwner({ agentPubkey }: { agentPubkey: string }) {
  const { query, tree } = useAgentFileTree(agentPubkey);
  const [selectedPath, setSelectedPath] = React.useState<string | null>(null);
  const selected =
    query.data?.files.find((file) => file.path === selectedPath) ?? null;

  return (
    <section data-testid="agent-files-section">
      {query.isLoading && !query.data ? <FilesSkeleton /> : null}

      {query.isError && !query.data ? (
        <FilesErrorState
          error={query.error}
          onRetry={() => query.refetch()}
          retrying={query.isFetching}
        />
      ) : null}

      {query.data && tree ? (
        selected ? (
          <FileView
            agentPubkey={agentPubkey}
            edit={query.data.edits.find((e) => e.path === selected.path)}
            file={selected}
            onBack={() => setSelectedPath(null)}
          />
        ) : tree.length === 0 ? (
          <FilesEmptyState />
        ) : (
          <FileTree
            edits={query.data.edits}
            nodes={tree}
            onOpen={setSelectedPath}
          />
        )
      ) : null}
    </section>
  );
}

// ── Subviews ────────────────────────────────────────────────────────────────

function FilesSkeleton() {
  return (
    <div
      aria-label="Loading files"
      className="space-y-2 p-4"
      data-testid="agent-files-skeleton"
      role="status"
    >
      <Skeleton className="h-4 w-2/3" />
      <Skeleton className="h-4 w-1/2" />
      <Skeleton className="h-4 w-3/5" />
    </div>
  );
}

function FilesErrorState({
  error,
  onRetry,
  retrying,
}: {
  error: unknown;
  onRetry: () => void;
  retrying: boolean;
}) {
  const message =
    error instanceof Error ? error.message : String(error ?? "unknown error");
  return (
    <div
      className="m-3 flex flex-col gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-xs"
      data-testid="agent-files-error"
      role="alert"
    >
      <div className="flex items-start gap-2">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
        <div className="space-y-1">
          <div className="font-medium text-destructive">
            Couldn't load files
          </div>
          <div className="text-muted-foreground">{message}</div>
        </div>
      </div>
      <Button
        className="self-start"
        disabled={retrying}
        onClick={onRetry}
        size="sm"
        variant="outline"
      >
        {retrying ? "Retrying…" : "Retry"}
      </Button>
    </div>
  );
}

function FilesEmptyState() {
  return (
    <div
      className="flex min-h-56 flex-col items-center justify-center px-6 py-10 text-center"
      data-testid="agent-files-empty"
    >
      <FolderOpen className="mx-auto h-4 w-4 text-muted-foreground" />
      <p className="mt-3 text-sm font-medium">No shared files</p>
      <p className="mt-1 text-sm text-muted-foreground">
        Add files or folders under Shared files in this agent's settings.
      </p>
    </div>
  );
}

function FileTree({
  edits,
  nodes,
  onOpen,
}: {
  edits: AgentFileEdit[];
  nodes: FileTreeNode[];
  onOpen: (path: string) => void;
}) {
  const [collapsed, setCollapsed] = React.useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const pendingPaths = React.useMemo(
    () =>
      new Set(edits.filter((e) => e.status === "pending").map((e) => e.path)),
    [edits],
  );
  const toggle = (path: string) =>
    setCollapsed((current) => {
      const next = new Set(current);
      if (!next.delete(path)) next.add(path);
      return next;
    });

  const rows: React.ReactNode[] = [];
  const walk = (level: FileTreeNode[], depth: number) => {
    for (const node of level) {
      const indent = { paddingLeft: `${1 + depth * 1.25}rem` };
      if (node.kind === "folder") {
        const open = !collapsed.has(node.path);
        rows.push(
          <button
            aria-expanded={open}
            className="flex w-full items-center gap-2 py-2 pr-4 text-left text-sm transition-colors hover:bg-muted/50"
            data-testid="agent-files-folder"
            key={node.path}
            onClick={() => toggle(node.path)}
            style={indent}
            type="button"
          >
            <ChevronRight
              className={cn(
                "h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform",
                open && "rotate-90",
              )}
            />
            <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate font-medium">
              {node.name}
            </span>
          </button>,
        );
        if (open) walk(node.children, depth + 1);
      } else {
        rows.push(
          <button
            className="flex w-full items-center gap-2 py-2 pr-4 text-left text-sm transition-colors hover:bg-muted/50"
            data-testid="agent-files-file"
            key={node.path}
            onClick={() => onOpen(node.path)}
            style={indent}
            type="button"
          >
            <span className="w-3.5 shrink-0" />
            <FileText className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate">{node.name}</span>
            {pendingPaths.has(node.path) ? (
              <span className="shrink-0 text-2xs text-warning">
                Edit pending
              </span>
            ) : (
              <span className="shrink-0 text-2xs text-muted-foreground">
                {formatFileSize(node.file.size)}
              </span>
            )}
          </button>,
        );
      }
    }
  };
  walk(nodes, 0);

  return (
    <div className="py-1" data-testid="agent-files-tree">
      {rows}
    </div>
  );
}

function FileView({
  agentPubkey,
  edit,
  file,
  onBack,
}: {
  agentPubkey: string;
  edit: AgentFileEdit | undefined;
  file: AgentFileEntry;
  onBack: () => void;
}) {
  const propose = useProposeAgentFileEdit(agentPubkey);
  const [draft, setDraft] = React.useState<string | null>(null);
  const editable = file.content !== null && edit?.status !== "pending";

  const submit = async () => {
    if (draft === null) return;
    try {
      await propose.mutateAsync({
        path: file.path,
        baseSha256: file.sha256,
        content: draft,
      });
      setDraft(null);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Couldn't send your edit",
      );
    }
  };

  return (
    <div data-testid="agent-files-view">
      <div className="flex min-h-12 items-center gap-1 border-border/55 border-b px-2 py-2">
        <Button
          aria-label="Back to files"
          onClick={onBack}
          size="icon"
          type="button"
          variant="ghost"
        >
          <ChevronLeft className="h-4 w-4" />
        </Button>
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-foreground">
          {file.path}
        </span>
        <span className="shrink-0 px-2 text-2xs text-muted-foreground">
          {formatFileSize(file.size)}
        </span>
        {editable && draft === null ? (
          <Button
            data-testid="agent-files-edit"
            onClick={() => setDraft(file.content)}
            size="sm"
            type="button"
            variant="outline"
          >
            Edit
          </Button>
        ) : null}
      </div>

      {edit && draft === null ? (
        <EditStatusBanner
          edit={edit}
          onEditAgain={
            file.content !== null ? () => setDraft(edit.content) : undefined
          }
        />
      ) : null}

      {draft !== null ? (
        <div className="space-y-3 p-4">
          <Textarea
            aria-label={`Edit ${file.path}`}
            className="min-h-72 font-mono text-xs leading-relaxed"
            data-testid="agent-files-editor"
            onChange={(event) => setDraft(event.target.value)}
            value={draft}
          />
          <p className="text-xs text-muted-foreground">
            The agent applies your edit if the file hasn't changed since you
            opened it.
          </p>
          <div className="flex justify-end gap-2">
            <Button
              disabled={propose.isPending}
              onClick={() => setDraft(null)}
              size="sm"
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              data-testid="agent-files-propose"
              disabled={propose.isPending || draft === file.content}
              onClick={submit}
              size="sm"
              type="button"
            >
              {propose.isPending ? "Sending…" : "Propose edit"}
            </Button>
          </div>
        </div>
      ) : (
        <FileContent file={file} />
      )}
    </div>
  );
}

function FileContent({ file }: { file: AgentFileEntry }) {
  if (file.content === null) {
    return (
      <p
        className="px-4 py-6 text-sm text-muted-foreground"
        data-testid="agent-files-unavailable"
      >
        {file.size > INLINE_LIMIT_BYTES
          ? "Too large to preview. Agents share the text of files up to 64 KB."
          : "Preview unavailable — this isn't a text file."}
      </p>
    );
  }

  const language = languageForPath(file.path);
  if (language === "markdown") {
    return (
      <div className="px-4 py-3" data-testid="agent-files-content">
        <Markdown
          className="text-sm leading-6"
          content={file.content}
          interactive={false}
        />
      </div>
    );
  }

  return (
    <pre
      className="overflow-x-auto bg-background/60 p-4"
      data-testid="agent-files-content"
    >
      {language ? (
        <SyntaxHighlightedCode
          className="whitespace-pre-wrap break-words text-xs leading-relaxed"
          code={file.content}
          language={language}
        />
      ) : (
        <code className="block min-w-full whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-foreground">
          {file.content}
        </code>
      )}
    </pre>
  );
}

function EditStatusBanner({
  edit,
  onEditAgain,
}: {
  edit: AgentFileEdit;
  onEditAgain?: () => void;
}) {
  const tone =
    edit.status === "conflict" || edit.status === "declined"
      ? "border-warning/30 bg-warning/5"
      : "border-border/55 bg-muted/30";

  return (
    <div
      className={cn(
        "mx-3 mt-3 flex items-start gap-2 rounded-md border px-3 py-2 text-xs",
        tone,
      )}
      data-status={edit.status}
      data-testid="agent-files-edit-status"
      role="status"
    >
      {edit.status === "pending" ? (
        <Loader2 className="mt-0.5 h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
      ) : edit.status === "applied" ? null : (
        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warning" />
      )}
      <span className="flex-1 text-muted-foreground">
        {edit.status === "pending"
          ? "Your edit is waiting for the agent."
          : edit.status === "applied"
            ? `The agent applied your edit ${relativeTime(edit.answeredAt ?? edit.createdAt)}.`
            : edit.status === "conflict"
              ? "The file changed before your edit reached the agent, so nothing was written."
              : `The agent declined your edit${edit.reason ? `: ${edit.reason}` : "."}`}
      </span>
      {edit.status === "conflict" && onEditAgain ? (
        <button
          className="shrink-0 font-medium text-warning hover:underline"
          onClick={onEditAgain}
          type="button"
        >
          Edit again
        </button>
      ) : null}
    </div>
  );
}
