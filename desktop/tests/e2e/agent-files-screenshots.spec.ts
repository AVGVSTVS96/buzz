import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import {
  installMockBridge,
  type MockAgentFilesListing,
  TEST_IDENTITIES,
} from "../helpers/bridge";

const SCREENSHOT_DIR = "test-results/agent-files";
const agentPubkey = TEST_IDENTITIES.charlie.pubkey;

function file(
  path: string,
  content: string | null,
  size = content?.length ?? 0,
) {
  return {
    path,
    sha256: `${path.length.toString(16).padStart(2, "0")}`.repeat(32),
    size,
    content,
    eventId: `event-${path}`,
    createdAt: 1_700_000_000,
  };
}

const agentFiles: MockAgentFilesListing = {
  files: [
    file("AGENTS.md", "# Charlie\n\nIndexes the channel catalog.\n"),
    file(
      "PLANS/catalog-refresh.md",
      "# Catalog refresh\n\n- [x] Index #agents\n- [ ] Re-rank stale entries\n- [ ] Post a summary to #general\n",
    ),
    file("PLANS/archive/q3.md", "# Q3\n\nShipped.\n"),
    file("notes/scratch.ts", 'export const lastRun = "2026-10-06";\n'),
    file("logs/indexer.log", null, 1_482_113),
  ],
  edits: [],
  truncated: false,
  fetchedAt: 1_700_000_000,
};

async function openCharlieFiles(page: Page) {
  await installMockBridge(page, {
    agentFiles,
    managedAgents: [
      {
        channelNames: ["agents"],
        name: "Charlie",
        pubkey: agentPubkey,
        status: "running",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-agents").click();
  const messageRow = page
    .getByTestId("message-row")
    .filter({ hasText: "Indexing the channel catalog now." });
  await messageRow.locator("button").first().click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
  await page.getByTestId("user-profile-tab-memories").click();
  await expect(page.getByTestId("user-profile-open-files")).toContainText("5");
  await capture(page, "00-memories-tab");
  await page.getByTestId("user-profile-open-files").click();
  await expect(page.getByTestId("agent-files-tree")).toBeVisible();
}

async function capture(page: Page, name: string) {
  await waitForAnimations(page);
  await page
    .getByTestId("user-profile-panel")
    .screenshot({ path: `${SCREENSHOT_DIR}/${name}.png` });
}

test("files tab shows the shared tree, a rendered file, and an edit proposal", async ({
  page,
}) => {
  await openCharlieFiles(page);
  await expect(page.getByTestId("agent-files-folder")).toHaveCount(4);
  await capture(page, "01-files-tree");

  await page
    .getByTestId("agent-files-file")
    .filter({ hasText: "catalog-refresh.md" })
    .click();
  await expect(page.getByTestId("agent-files-content")).toContainText(
    "Re-rank stale entries",
  );
  await capture(page, "02-markdown-file");

  await page.getByTestId("agent-files-edit").click();
  const editor = page.getByTestId("agent-files-editor");
  await editor.fill(
    "# Catalog refresh\n\n- [x] Index #agents\n- [x] Re-rank stale entries\n- [ ] Post a summary to #general\n",
  );
  await capture(page, "03-editing");

  await page.getByTestId("agent-files-propose").click();
  await expect(page.getByTestId("agent-files-edit-status")).toHaveAttribute(
    "data-status",
    "pending",
  );
  await capture(page, "04-edit-pending");

  await page.getByRole("button", { name: "Back to files" }).click();
  await page
    .getByTestId("agent-files-file")
    .filter({ hasText: "indexer.log" })
    .click();
  await expect(page.getByTestId("agent-files-unavailable")).toContainText(
    "Too large to preview",
  );
  await capture(page, "05-too-large");
});

test("an answered edit shows the agent's outcome", async ({ page }) => {
  agentFiles.edits = [
    {
      requestId: "f".repeat(64),
      path: "AGENTS.md",
      baseSha256: "0".repeat(64),
      content: "# Charlie\n",
      createdAt: Math.floor(Date.now() / 1000) - 120,
      status: "conflict",
      sha256: file("AGENTS.md", "").sha256,
      reason: null,
      answeredAt: Math.floor(Date.now() / 1000) - 60,
    },
  ];
  await openCharlieFiles(page);
  await page
    .getByTestId("agent-files-file")
    .filter({ hasText: "AGENTS.md" })
    .click();
  await expect(page.getByTestId("agent-files-edit-status")).toHaveAttribute(
    "data-status",
    "conflict",
  );
  await capture(page, "06-edit-conflict");
});
