import assert from "node:assert/strict";
import { test } from "node:test";

import { routeReviewKey } from "../src/lib/reviewKeys.ts";
import { jobAfterPoll } from "../src/lib/acquireJobs.ts";
import { candidateLabel, matchCandidateWork, nextCandidateIndex, normalizeDoi, normalizeTitle } from "../src/lib/claimCandidates.ts";

const hit = (over) => ({ work_id: "w1", paper_id: null, title: "Attention Is All You Need", year: 2017, authors: [], source: "library", in_graph: true, openalex_id: null, doi: null, ...over });
const candidate = (over) => ({ provider_id: "p", title: "Attention is all you need", year: 2017, authors: ["Vaswani"], doi: null, ...over });

test("should strip resolver prefixes and case when normalizing a DOI", () => {
  assert.equal(normalizeDoi("https://doi.org/10.1/ABC"), "10.1/abc");
  assert.equal(normalizeDoi("doi: 10.1/abc"), "10.1/abc");
  assert.equal(normalizeDoi("  "), null);
  assert.equal(normalizeDoi(null), null);
});

test("should fold accents and punctuation when normalizing a title", () => {
  assert.equal(normalizeTitle("Café: A Study!"), normalizeTitle("cafe a study"));
  assert.equal(normalizeTitle("???"), "");
});

test("should match by DOI when the title differs", () => {
  const found = matchCandidateWork(candidate({ title: "Other", doi: "10.1/X" }), [hit({ title: "Unrelated", doi: "https://doi.org/10.1/x" })]);
  assert.equal(found, "w1");
});

test("should match by title and year when no DOI is given", () => {
  assert.equal(matchCandidateWork(candidate(), [hit()]), "w1");
});

test("should not match when the years differ by more than one", () => {
  assert.equal(matchCandidateWork(candidate({ year: 2010 }), [hit()]), null);
});

test("should match when the years differ by one", () => {
  assert.equal(matchCandidateWork(candidate({ year: 2016 }), [hit()]), "w1");
});

test("should match by provider id when the hit carries the OpenAlex id", () => {
  assert.equal(matchCandidateWork(candidate({ provider_id: "W9", title: "Other" }), [hit({ title: "Unrelated", openalex_id: "W9" })]), "w1");
});

test("should match when either side has no year", () => {
  assert.equal(matchCandidateWork(candidate({ year: null }), [hit()]), "w1");
});

test("should ignore hits that are not Works yet when matching", () => {
  assert.equal(matchCandidateWork(candidate(), [hit({ work_id: null, source: "provider" })]), null);
});

test("should not match an empty title when both titles normalize to nothing", () => {
  assert.equal(matchCandidateWork(candidate({ title: "!!!" }), [hit({ title: "???" })]), null);
});

test("should return null when there are no hits", () => {
  assert.equal(matchCandidateWork(candidate(), []), null);
});

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
