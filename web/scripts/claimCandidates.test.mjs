import assert from "node:assert/strict";
import { test } from "node:test";

import { routeReviewKey } from "../src/lib/reviewKeys.ts";
import { jobAfterPoll } from "../src/lib/acquireJobs.ts";
import { candidateLabel, nextCandidateIndex } from "../src/lib/claimCandidates.ts";

const candidate = (over) => ({ provider_id: "p", title: "Attention is all you need", year: 2017, authors: ["Vaswani"], doi: null, ...over });

test("should label a candidate with year and first author when authors and year exist", () => {
  assert.equal(candidateLabel(candidate({ authors: ["A", "B"] })), "Attention is all you need · 2017 · A et al.");
  assert.equal(candidateLabel(candidate({ year: null, authors: [] })), "Attention is all you need");
});

test("should wrap the candidate index when cycling past the last one", () => {
  assert.equal(nextCandidateIndex(0, 3), 1);
  assert.equal(nextCandidateIndex(2, 3), 0);
  assert.equal(nextCandidateIndex(0, 0), 0);
});

test("should act on the cursor node when the selected link is already accepted", () => {
  assert.deepEqual(routeReviewKey("x", false, "settled"), { route: "node-reject", clearEdge: true });
  assert.deepEqual(routeReviewKey("a", false, "missing"), { route: "node-accept", clearEdge: true });
});

test("should act on the link when it is still proposed", () => {
  assert.deepEqual(routeReviewKey("x", false, "proposed"), { route: "edge-reject", clearEdge: true });
  assert.deepEqual(routeReviewKey("a", false, "proposed"), { route: "edge-accept", clearEdge: true });
});

test("should route chooser keys to the chooser when it is open", () => {
  assert.equal(routeReviewKey("Enter", true, "none").route, "chooser-resolve");
  assert.equal(routeReviewKey("Escape", true, "none").route, "chooser-close");
  assert.equal(routeReviewKey("c", true, "none").route, "chooser-next");
  assert.equal(routeReviewKey("c", false, "none").route, "node-resolve");
});

test("should ignore other keys when nothing claims them", () => {
  assert.deepEqual(routeReviewKey("q", false, "proposed"), { route: "ignore", clearEdge: false });
  assert.equal(routeReviewKey("Enter", false, "none").route, "ignore");
});

test("should keep polling when the fetch job is running", () => {
  assert.deepEqual(jobAfterPoll("running", null), { finished: false, job: { state: "running", message: null } });
});

test("should show the server message when the fetch job failed", () => {
  assert.deepEqual(jobAfterPoll("failed", "no pdf"), { finished: true, job: { state: "failed", message: "no pdf" } });
  assert.deepEqual(jobAfterPoll("failed", undefined), { finished: true, job: { state: "failed", message: null } });
});

test("should clear the job when the fetch job is idle or done", () => {
  assert.deepEqual(jobAfterPoll("idle", null), { finished: true, job: null });
  assert.deepEqual(jobAfterPoll("succeeded", null), { finished: true, job: null });
});
