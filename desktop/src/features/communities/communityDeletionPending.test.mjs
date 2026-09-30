import assert from "node:assert/strict";
import test from "node:test";

import {
  clearPendingCommunityDeletion,
  deletionResponseDisposition,
  loadPendingCommunityDeletion,
  persistPendingCommunityDeletion,
  pendingCommunityDeletionMatchesAccount,
  pendingCommunityDeletionMatchesPersisted,
  pendingCommunityDeletionForAccount,
  publicDeletionRequest,
} from "./communityDeletionPending.ts";

function storage() {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
    values,
  };
}

const envelope = {
  community_id: "4efb8c89-b9cb-4a26-863d-cf2bd5f9d5c1",
  host: "Exact-Host.communities.buzz.xyz",
  request_id: "b2456816-eea0-4f74-9c54-531645dbaec9",
  acknowledgement_version: 1,
  bound_owner_pubkey: "a".repeat(64),
  backend_origin: "https://app.builderlab.xyz",
};

function transport(http_status, body) {
  return { http_status, body };
}

test("pending deletion round-trips exact host bytes and account binding", () => {
  const target = storage();
  assert.equal(persistPendingCommunityDeletion(envelope, target), true);
  assert.deepEqual(loadPendingCommunityDeletion(target), envelope);
  assert.equal(
    pendingCommunityDeletionMatchesAccount(
      envelope,
      "a".repeat(64),
      "https://app.builderlab.xyz",
    ),
    true,
  );
  assert.equal(
    pendingCommunityDeletionMatchesAccount(
      envelope,
      "b".repeat(64),
      "https://app.builderlab.xyz",
    ),
    false,
  );
});

test("single-slot A-B-A view retains exact bytes and blocks another owner", () => {
  const target = storage();
  const bytes = JSON.stringify(envelope);
  target.setItem("buzz:hosted-community-delete-pending:v1", bytes);
  assert.deepEqual(pendingCommunityDeletionForAccount("a".repeat(64), target), {
    owned: envelope,
    blockingOwnerPubkey: null,
  });
  assert.deepEqual(pendingCommunityDeletionForAccount("b".repeat(64), target), {
    owned: null,
    blockingOwnerPubkey: "a".repeat(64),
  });
  assert.equal(
    target.getItem("buzz:hosted-community-delete-pending:v1"),
    bytes,
  );
  assert.equal(
    persistPendingCommunityDeletion(
      { ...envelope, bound_owner_pubkey: "b".repeat(64) },
      target,
    ),
    false,
  );
  assert.deepEqual(pendingCommunityDeletionForAccount("a".repeat(64), target), {
    owned: envelope,
    blockingOwnerPubkey: null,
  });
  assert.deepEqual(publicDeletionRequest(envelope), {
    community_id: envelope.community_id,
    host: envelope.host,
    request_id: envelope.request_id,
    acknowledgement_version: envelope.acknowledgement_version,
  });
  assert.equal(
    target.getItem("buzz:hosted-community-delete-pending:v1"),
    bytes,
  );
});

test("pending deletion rejects repaired hosts, malformed UUIDs, and unknown fields", () => {
  const target = storage();
  for (const invalid of [
    { ...envelope, host: ` ${envelope.host}` },
    { ...envelope, request_id: envelope.request_id.toUpperCase() },
    { ...envelope, acknowledgement_version: 2 },
    { ...envelope, extra: true },
  ]) {
    target.setItem(
      "buzz:hosted-community-delete-pending:v1",
      JSON.stringify(invalid),
    );
    assert.equal(loadPendingCommunityDeletion(target), null);
    assert.equal(target.values.size, 0, "invalid envelopes are discarded");
  }
});

test("persistence failure is observable and clear is bounded to the deletion key", () => {
  const throwing = {
    getItem: () => null,
    setItem: () => {
      throw new Error("denied");
    },
    removeItem: () => {},
  };
  assert.equal(persistPendingCommunityDeletion(envelope, throwing), false);

  const target = storage();
  target.setItem("unrelated", "keep");
  assert.equal(persistPendingCommunityDeletion(envelope, target), true);
  clearPendingCommunityDeletion(envelope, target);
  assert.equal(target.getItem("unrelated"), "keep");
});

test("persistence boundary never overwrites an existing envelope", () => {
  const target = storage();
  assert.equal(persistPendingCommunityDeletion(envelope, target), true);
  const second = {
    ...envelope,
    community_id: "33333333-3333-4333-8333-333333333333",
    request_id: "44444444-4444-4444-8444-444444444444",
  };
  assert.equal(persistPendingCommunityDeletion(second, target), false);
  assert.deepEqual(loadPendingCommunityDeletion(target), envelope);
});

test("persisted request must still match all tuple and owner/origin fields before dispatch", () => {
  const target = storage();
  assert.equal(persistPendingCommunityDeletion(envelope, target), true);
  assert.equal(
    pendingCommunityDeletionMatchesPersisted(envelope, target),
    true,
  );
  for (const changed of [
    { ...envelope, request_id: "44444444-4444-4444-8444-444444444444" },
    { ...envelope, community_id: "33333333-3333-4333-8333-333333333333" },
    { ...envelope, host: "other.communities.buzz.xyz" },
    { ...envelope, bound_owner_pubkey: "b".repeat(64) },
    { ...envelope, backend_origin: "https://other.example" },
  ]) {
    assert.equal(
      pendingCommunityDeletionMatchesPersisted(changed, target),
      false,
    );
  }
  clearPendingCommunityDeletion(envelope, target);
  assert.equal(
    pendingCommunityDeletionMatchesPersisted(envelope, target),
    false,
  );
});

test("terminal clear affects only the matching request and account envelope", () => {
  const target = storage();
  const second = {
    ...envelope,
    request_id: "44444444-4444-4444-8444-444444444444",
  };
  assert.equal(persistPendingCommunityDeletion(second, target), true);
  const bytes = target.getItem("buzz:hosted-community-delete-pending:v1");
  clearPendingCommunityDeletion(envelope, target);
  assert.equal(
    target.getItem("buzz:hosted-community-delete-pending:v1"),
    bytes,
  );
  clearPendingCommunityDeletion(second, target);
  assert.equal(loadPendingCommunityDeletion(target), null);
});

test("same-UUID check settles definitive errors but retains fresh-only and wrong status", () => {
  for (const [code, status, freshOnly] of [
    ["missing_mapping", 400, true],
    ["invalid_request", 400, true],
    ["confirmation_mismatch", 400, true],
    ["unsupported_acknowledgement_version", 400, true],
    ["not_owner", 404, false],
    ["must_archive", 409, false],
    ["protected_target", 409, false],
    ["deletion_conflict", 409, false],
  ]) {
    for (const attempt of ["initial", "check"]) {
      assert.equal(
        deletionResponseDisposition(
          transport(status, { error: { code } }),
          envelope,
          attempt,
        ),
        attempt === "check" && freshOnly ? "retain" : "clear",
        `${attempt} ${code} exact status`,
      );
      assert.equal(
        deletionResponseDisposition(
          transport(status + 1, { error: { code } }),
          envelope,
          attempt,
        ),
        "retain",
        `${attempt} ${code} wrong status`,
      );
    }
  }
  assert.equal(
    deletionResponseDisposition(
      transport(409, { error: { code: "deletion_request_conflict" } }),
      envelope,
      "check",
    ),
    "retain",
  );
  assert.equal(
    deletionResponseDisposition(
      transport(503, { error: { code: "acceptance_unknown" } }),
      envelope,
      "initial",
    ),
    "retain",
  );
});

test("only tuple-bound canonical stages or abort settle same-UUID recovery", () => {
  const tuple = {
    request_id: envelope.request_id,
    community_id: envelope.community_id,
    host: envelope.host,
    acknowledgement_version: envelope.acknowledgement_version,
  };
  for (const status of [
    "submitted",
    "inventoried",
    "approved",
    "fenced",
    "drained",
    "bindings_removed",
    "postgres_purged",
    "cache_purged",
    "logically_verified",
    "retention_pending",
  ]) {
    assert.equal(
      deletionResponseDisposition(
        transport(202, { ...tuple, status }),
        envelope,
        "check",
      ),
      "accept",
      status,
    );
  }
  assert.equal(
    deletionResponseDisposition(
      transport(202, { ...tuple, status: "aborted" }),
      envelope,
      "check",
    ),
    "abort",
  );
  assert.equal(
    deletionResponseDisposition(
      transport(409, { ...tuple, error: { code: "deletion_aborted" } }),
      envelope,
      "check",
    ),
    "abort",
  );
  assert.equal(
    deletionResponseDisposition(
      transport(409, { error: { code: "deletion_aborted" } }),
      envelope,
      "check",
    ),
    "retain",
  );
  for (const status of ["accepted", "admitted", "completed", "future_stage"]) {
    assert.equal(
      deletionResponseDisposition(
        transport(202, { ...tuple, status }),
        envelope,
        "check",
      ),
      "retain",
      status,
    );
  }
  for (const body of [
    {
      ...tuple,
      request_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      status: "approved",
    },
    { ...tuple, host: "other.communities.buzz.xyz", status: "approved" },
    { ...tuple, acknowledgement_version: 2, status: "aborted" },
  ]) {
    assert.equal(
      deletionResponseDisposition(transport(202, body), envelope, "check"),
      "retain",
    );
  }
  assert.equal(
    deletionResponseDisposition(
      transport(503, { ...tuple, status: "approved", http_status: 202 }),
      envelope,
      "initial",
    ),
    "retain",
  );
  assert.equal(
    deletionResponseDisposition(
      transport(200, {
        ...tuple,
        error: { code: "deletion_aborted" },
        http_status: 409,
      }),
      envelope,
      "check",
    ),
    "retain",
  );
});

test("a typed rejection with a partial or mismatched tuple stays uncertain", () => {
  for (const attempt of ["initial", "check"]) {
    assert.equal(
      deletionResponseDisposition(
        transport(404, {
          error: { code: "not_owner" },
          request_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        }),
        envelope,
        attempt,
      ),
      "retain",
    );
    assert.equal(
      deletionResponseDisposition(
        transport(409, {
          error: { code: "must_archive" },
          host: "elsewhere.example",
        }),
        envelope,
        attempt,
      ),
      "retain",
    );
  }
  assert.equal(
    deletionResponseDisposition(
      transport(400, { error: { code: "invalid_request" }, request_id: 42 }),
      envelope,
      "initial",
    ),
    "retain",
  );
  assert.equal(
    deletionResponseDisposition(transport(409, null), envelope, "check"),
    "retain",
  );
});

test("fresh admission clears only the established exact code and native-status pairs", () => {
  const terminalPairs = [
    ["missing_mapping", 400],
    ["invalid_request", 400],
    ["confirmation_mismatch", 400],
    ["unsupported_acknowledgement_version", 400],
    ["not_owner", 404],
    ["must_archive", 409],
    ["protected_target", 409],
    ["deletion_conflict", 409],
  ];
  for (const [code, httpStatus] of terminalPairs) {
    assert.equal(
      deletionResponseDisposition(
        transport(httpStatus, { error: { code } }),
        envelope,
        "initial",
      ),
      "clear",
      `${code} requires HTTP ${httpStatus}`,
    );
    assert.equal(
      deletionResponseDisposition(
        transport(httpStatus + 1, { error: { code } }),
        envelope,
        "initial",
      ),
      "retain",
      `${code} with the wrong native status is ambiguous`,
    );
  }

  for (const code of [
    "acceptance_unknown",
    "unauthorized",
    "relay_unavailable",
    "unknown",
    "future_code",
  ]) {
    assert.equal(
      deletionResponseDisposition(
        transport(400, { error: { code } }),
        envelope,
        "initial",
      ),
      "retain",
      `${code} cannot clear the envelope`,
    );
  }
});

test("native status, never a body-claimed status, binds acceptance and abort", () => {
  const tuple = {
    request_id: envelope.request_id,
    community_id: envelope.community_id,
    host: envelope.host,
    acknowledgement_version: envelope.acknowledgement_version,
  };
  assert.equal(
    deletionResponseDisposition(
      transport(202, { ...tuple, status: "approved" }),
      envelope,
      "initial",
    ),
    "accept",
  );
  assert.equal(
    deletionResponseDisposition(
      transport(503, {
        ...tuple,
        status: "approved",
        http_status: 202,
      }),
      envelope,
      "initial",
    ),
    "retain",
  );
  assert.equal(
    deletionResponseDisposition(
      transport(409, { ...tuple, error: { code: "deletion_aborted" } }),
      envelope,
      "check",
    ),
    "abort",
  );
  assert.equal(
    deletionResponseDisposition(
      transport(200, {
        ...tuple,
        error: { code: "deletion_aborted" },
        http_status: 409,
      }),
      envelope,
      "check",
    ),
    "retain",
  );
});

test("missing native status cannot settle a typed deletion error", () => {
  for (const code of ["future_code", "not_owner", "must_archive"]) {
    for (const attempt of ["initial", "check"]) {
      assert.equal(
        deletionResponseDisposition(
          { body: { error: { code } } },
          envelope,
          attempt,
        ),
        "retain",
        `${attempt} ${code} without native status is ambiguous`,
      );
    }
  }
});
