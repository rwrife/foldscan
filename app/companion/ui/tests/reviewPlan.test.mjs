import assert from "node:assert/strict";
import test from "node:test";

import {
  createExportPreviewRequest,
  createReviewPlan,
  moveCapture,
  removeCapture,
  restoreCapture,
} from "../src/reviewPlan.ts";

const source = Object.freeze({
  sessions: [
    {
      session_id: "sess-a",
      captures: [
        { capture_id: "cap-1", bytes: 10 },
        { capture_id: "cap-2", bytes: 20 },
        { capture_id: "cap-3", bytes: 30 },
      ],
    },
    {
      session_id: "sess-b",
      captures: [{ capture_id: "cap-4", bytes: 40 }],
    },
  ],
});

function ids(plan, sessionId) {
  return plan.sessions
    .find((session) => session.sessionId === sessionId)
    .active.map((capture) => capture.captureId);
}

test("a review plan preserves manifest order for every session", () => {
  const plan = createReviewPlan(source);
  assert.deepEqual(ids(plan, "sess-a"), ["cap-1", "cap-2", "cap-3"]);
  assert.deepEqual(ids(plan, "sess-b"), ["cap-4"]);
});

test("an export preview request preserves the reviewed active order and volume path", () => {
  let plan = createReviewPlan(source);
  plan = moveCapture(plan, "sess-a", "cap-3", -1);
  plan = removeCapture(plan, "sess-a", "cap-1");
  const session = plan.sessions.find((entry) => entry.sessionId === "sess-a");

  assert.deepEqual(createExportPreviewRequest("/media/FOLDSCAN", session), {
    volumePath: "/media/FOLDSCAN",
    sessionId: "sess-a",
    captureIds: ["cap-3", "cap-2"],
  });
});

test("moving a capture is immutable and clamped at both boundaries", () => {
  const plan = createReviewPlan(source);

  assert.deepEqual(ids(moveCapture(plan, "sess-a", "cap-2", -1), "sess-a"), [
    "cap-2",
    "cap-1",
    "cap-3",
  ]);
  assert.deepEqual(ids(moveCapture(plan, "sess-a", "cap-2", 1), "sess-a"), [
    "cap-1",
    "cap-3",
    "cap-2",
  ]);
  assert.deepEqual(ids(moveCapture(plan, "sess-a", "cap-1", -1), "sess-a"), [
    "cap-1",
    "cap-2",
    "cap-3",
  ]);
  assert.deepEqual(ids(moveCapture(plan, "sess-a", "cap-3", 1), "sess-a"), [
    "cap-1",
    "cap-2",
    "cap-3",
  ]);
  assert.deepEqual(ids(plan, "sess-a"), ["cap-1", "cap-2", "cap-3"]);
});

test("removal keeps originals in a separate source-ordered list", () => {
  const plan = createReviewPlan(source);
  const removed = removeCapture(removeCapture(plan, "sess-a", "cap-3"), "sess-a", "cap-1");

  assert.deepEqual(ids(removed, "sess-a"), ["cap-2"]);
  assert.deepEqual(
    removed.sessions[0].removed.map((entry) => entry.capture.captureId),
    ["cap-1", "cap-3"],
  );
  assert.deepEqual(ids(plan, "sess-a"), ["cap-1", "cap-2", "cap-3"]);
});

test("restoring a capture returns it to its previous export index or nearest boundary", () => {
  let plan = createReviewPlan(source);
  plan = removeCapture(plan, "sess-a", "cap-2"); // export index was 1
  plan = moveCapture(plan, "sess-a", "cap-3", -1);
  assert.deepEqual(ids(plan, "sess-a"), ["cap-3", "cap-1"]);

  plan = restoreCapture(plan, "sess-a", "cap-2"); // restored at index 1
  assert.deepEqual(ids(plan, "sess-a"), ["cap-3", "cap-2", "cap-1"]);
  assert.deepEqual(plan.sessions[0].removed, []);
});

test("review operations do not mutate the imported summary source", () => {
  const plan = createReviewPlan(source);
  removeCapture(moveCapture(plan, "sess-a", "cap-3", -1), "sess-a", "cap-1");

  assert.deepEqual(
    source.sessions[0].captures.map((capture) => capture.capture_id),
    ["cap-1", "cap-2", "cap-3"],
  );
  assert.equal(source.sessions[0].captures.length, 3);
});
