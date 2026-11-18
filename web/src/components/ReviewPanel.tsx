import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";

import "./ReviewPanel.css";
import { ApiError, request } from "../lib/api";
import { edgeRelationLabel, engagementLabel, pathLabel } from "../lib/edgeLabels";
import {
  GENERAL_COMPONENT,
  OVERALL,
  applyGradingKey,
  completion,
  draftFromReview,
  edgeLabelsPin,
  emptyDraft,
  focusRows,
  gradeOf,
  isProduced,
  nextIssueId,
  progressCounts,
  progressState,
  readAbstract,
  shownFields,
  readContext,
  readEdgeLabels,
  readLogical,
  readOutline,
  readThesis,
  setRowValue,
  setVerdict,
  toInput,
  updateGrade,
  valueOf,
  verdictItems,
  verdictOf,
  visibleDimensions,
  type EdgeLabelsPin,
  type FocusRow,
  type IssueFlag,
  type Manifest,
  type ManifestPaper,
  type PaperReview,
  type ProgressRow,
  type Review,
  type ReviewDraft,
  type ReviewInput,
  type Rubric,
  type RubricComponent,
  type RubricDimension,
  type ReviewedFields,
} from "../lib/review";
import { basisLabel, downgradeText, outlineLabel, relationLabel, roleLabel } from "../lib/readerOutput";
import type { ContextBasis, ContextClaim } from "../types";

type ReviewPanelProps = {
  paperId: string | null;
  onClose: () => void;
  onOpenPaper: (id: string) => void;
};

type SaveState = { kind: "idle" } | { kind: "saving" } | { kind: "saved"; at: string } | { kind: "error"; message: string } | { kind: "stale"; message: string };

const reviewApi = {
  rubric: (): Promise<Rubric> => request("/api/review/rubric"),
  manifest: (): Promise<Manifest> => request("/api/review/manifest"),
  progress: (): Promise<ProgressRow[]> => request("/api/review/progress"),
  paper: (id: string): Promise<PaperReview> => request(`/api/papers/${id}/review`),
  save: (id: string, input: ReviewInput): Promise<Review> =>
    request(`/api/papers/${id}/review`, { method: "PUT", body: JSON.stringify(input) }),
};

const AUTOSAVE_DELAY_MS = 700;

function rowKey(row: FocusRow): string {
  return `${row.componentId}.${row.dimension.id}`;
}

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement;
}

function NotProduced() {
  return <p className="review-not-produced">Not produced</p>;
}

function BasisTag({ basis }: { basis: ContextBasis }) {
  const label = basisLabel(basis);
  return <span className={`review-basis basis-${basis}`} title={label.long}>{label.short}</span>;
}

function ClaimsBlock({ title, claims }: { title: string; claims: ContextClaim[] }) {
  return <>
    <h4>{title}</h4>
    {claims.length === 0 ? <NotProduced /> : <ul className="review-claims">{claims.map((claim, index) => {
      const downgrade = downgradeText(claim);
      return <li key={index} data-basis={claim.basis}>
        <BasisTag basis={claim.basis} /> <span className="review-prose">{claim.text}</span>
        {claim.evidence.map((quote, quoteIndex) => <blockquote key={quoteIndex} className="review-quote">“{quote.quote}” <span className="review-muted">p. {quote.page}</span></blockquote>)}
        {claim.basis !== "paper" && claim.work_ids.length > 0 && <div className="review-muted">Works: {claim.work_ids.join(", ")}</div>}
        {downgrade !== null && <div className="review-muted">Downgraded to background: {downgrade}</div>}
      </li>;
    })}</ul>}
  </>;
}

/** Pipeline warnings about the fields a component grades, shown above its output. */
const WARNING_FIELDS: Record<string, string[]> = {
  abstract: ["abstract", "author_abstract"],
  thesis: ["thesis", "card"],
  context: ["context"],
  outline: ["outline"],
  logical: ["sections"],
};

/**
 * Warnings that belong to no single component, such as a skipped self-check
 * (`self_check`, `admission`, `repair`) or an implausible title (`identity`).
 * They affect everything below them, so they are shown once, above the components.
 */
function GeneralWarnings({ fields }: { fields: ReviewedFields }) {
  const prefixes = Object.values(WARNING_FIELDS).flat();
  const warnings = (fields.quality_warnings ?? []).filter((warning) => !prefixes.some((prefix) => warning.field.startsWith(prefix)));
  if (warnings.length === 0) return null;
  return <section className="review-general-warnings" aria-label="Pipeline warnings for the whole analysis">
    <h3>Pipeline warnings for the whole analysis</h3>
    <ul className="review-warnings">{warnings.map((warning, index) => (
      <li key={index}><code>{warning.field}</code> <code>{warning.kind}</code> {warning.message}</li>
    ))}</ul>
  </section>;
}

/** The dossier the context node read, in words. */
function dossierStatusText(status: string | undefined): string {
  switch (status) {
    case "ok": return "assembled from the citation graph";
    case "not_linked": return "empty: the paper is not linked to the citation graph";
    case "no_graph": return "empty: no citation graph is configured";
    case "error": return "empty: the citation graph could not be read";
    default: return "status unknown";
  }
}

function ComponentWarnings({ componentId, fields }: { componentId: string; fields: ReviewedFields }) {
  const prefixes = WARNING_FIELDS[componentId] ?? [];
  const warnings = (fields.quality_warnings ?? []).filter((warning) => prefixes.some((prefix) => warning.field.startsWith(prefix)));
  if (warnings.length === 0) return null;
  return <ul className="review-warnings" aria-label="Pipeline warnings">{warnings.map((warning, index) => (
    <li key={index}><code>{warning.kind}</code> {warning.message}</li>
  ))}</ul>;
}

function ComponentOutput({ component, fields, manifestPaper }: { component: RubricComponent; fields: ReviewedFields; manifestPaper: ManifestPaper | null }) {
  const body = (() => { switch (component.id) {
    case "abstract": {
      const text = readAbstract(fields);
      const f1 = fields.abstract_agreement?.token_f1;
      return <>
        {text === null ? <NotProduced /> : <p className="review-prose">{text}</p>}
        {f1 != null && <p className="review-hint">Agreement with the deterministic extraction: token F1 {f1.toFixed(2)}</p>}
        {manifestPaper?.oracle != null && <p className="review-hint">Reference abstract: <code>{manifestPaper.oracle}</code></p>}
      </>;
    }
    case "thesis": {
      const text = readThesis(fields);
      return text === null ? <NotProduced /> : <blockquote className="review-prose">{text}</blockquote>;
    }
    case "context": {
      const context = readContext(fields);
      const node = context.node;
      return <>
        {node === null ? <>
          <p className="review-hint">The context node did not run for this analysis; showing the earlier brief and notes.</p>
          {context.brief === null && context.legacyNotes.length === 0 ? <NotProduced /> : <>
            {context.brief !== null && <p className="review-prose">{context.brief}</p>}
            {context.legacyNotes.map((note, index) => <p className="review-prose" key={index}>
              {note.kind !== undefined && note.kind !== "legacy" && <strong>{note.kind === "before" ? "Before" : "After"}: </strong>}{note.text}
            </p>)}
          </>}
          {context.sources.length > 0 && <>
            <h4>Sources cited</h4>
            <ul className="review-list">{context.sources.map((source) => <li key={source.id}>
              <a href={source.url} target="_blank" rel="noreferrer">{source.title}</a>
              <div className="review-muted">{source.supports}</div>
            </li>)}</ul>
          </>}
        </> : <>
          <ClaimsBlock title="Why it matters" claims={node.lead} />
          <ClaimsBlock title="Before" claims={node.before} />
          <ClaimsBlock title="What it changed" claims={node.contribution} />
          <ClaimsBlock title="After" claims={node.after} />
          {node.reception_note != null && <p className="review-hint">Not known: {node.reception_note}</p>}
          <p className="review-hint">
            {node.counts.paper_claims} paper · {node.counts.dossier_claims} graph · {node.counts.background_claims} background claims;
            {" "}{node.counts.downgraded_quote_not_found + node.counts.downgraded_unknown_work} downgraded; dossier of {node.counts.dossier_works} works
            {node.dossier != null && <>, {dossierStatusText(node.dossier.status)}</>}
          </p>
          {node.dossier != null && node.dossier.works.length > 0 && <details className="review-expected">
            <summary>The dossier the model was given ({node.dossier.works.length} works)</summary>
            <ul className="review-list">{node.dossier.works.map((work) => <li key={work.id}>
              <code>{work.id}</code> <strong>{work.title ?? "(untitled)"}</strong>
              <span className="review-muted"> — {work.citing ? "cites this paper" : "cited by this paper"}{work.first_author == null ? "" : `, ${work.first_author}`}{work.year == null ? "" : `, ${work.year}`}</span>
            </li>)}</ul>
          </details>}
          <h4>Read alongside</h4>
          {node.read_alongside.length === 0 ? <NotProduced /> : <ul className="review-list">
            {node.read_alongside.map((item, index) => <li key={index}>
              <BasisTag basis={item.basis} /> <strong>{item.title}</strong>
              <span className="review-muted"> — {relationLabel(item.relation)}{item.authors == null ? "" : `, ${item.authors}`}{item.year == null ? "" : `, ${item.year}`}
                {item.local_paper_id != null ? " · in library" : item.work_id != null ? " · in graph" : ""}</span>
              <div>{item.why}</div>
            </li>)}
          </ul>}
        </>}
        {manifestPaper !== null && manifestPaper.neighbours.length > 0 && <details className="review-expected">
          <summary>Evaluation-set neighbours ({manifestPaper.neighbours.length}) — what you might expect</summary>
          <ul className="review-list">{manifestPaper.neighbours.map((neighbour) => <li key={neighbour.id}>
            <strong>{neighbour.title}</strong>
            <span className="review-muted"> — {neighbour.relation}{neighbour.year === undefined ? "" : `, ${neighbour.year}`}</span>
            <div>{neighbour.why}</div>
          </li>)}</ul>
        </details>}
      </>;
    }
    case "outline": {
      const rows = readOutline(fields);
      return rows.length === 0 ? <NotProduced /> : <ol className="review-outline">
        {rows.map((row) => <li key={row.index} data-role={row.entry.role ?? "main"} style={{ paddingLeft: `${row.depth * 18}px` }}>
          <span>{outlineLabel(row.entry)}</span>
          <span className="review-muted"> p. {row.entry.page}{(row.entry.role ?? "main") !== "main" ? ` · ${roleLabel(row.entry.role)}` : ""}</span>
        </li>)}
      </ol>;
    }
    case "logical": {
      const logical = readLogical(fields);
      if (logical.units.length === 0) return <NotProduced />;
      return <>
        {!logical.grouped && <p className="review-hint">These units were cut by page span, without a usable printed outline.</p>}
        <ol className="review-units">{logical.units.map(({ section, range, covers }) => <li key={section.id} data-family={section.family}>
          <div><strong>{section.title}</strong><span className="review-muted"> {section.pages.start === section.pages.end ? `p. ${section.pages.start}` : `pp. ${section.pages.start}–${section.pages.end}`} · {section.kind}</span></div>
          {range !== null && <div className="review-muted">Groups {range}: {covers.join(" · ")}</div>}
          {section.question != null && section.question !== "" && <div className="review-question">{section.question}</div>}
          {section.summary !== "" && <p>{section.summary}</p>}
        </li>)}</ol>
      </>;
    }
    case "edge_labels": {
      const labels = readEdgeLabels(fields);
      if (labels.pinned === null) return <NotProduced />;
      return <>
        <p className="review-hint">
          {labels.pinned.label_version}
          {labels.pinned.status !== "completed" && labels.pinned.detail !== null && <> · {labels.pinned.status}: {labels.pinned.detail}</>}
          {labels.countsLine !== null && <> · {labels.countsLine}</>}
        </p>
        {labels.sample.length === 0 ? <NotProduced /> : <ol className="review-edge-labels">{labels.sample.map((edge) => {
          const label = edge.label;
          if (label === null) return null;
          const engagement = engagementLabel(label.engagement);
          return <li key={label.claim_id} data-relation={label.relation} data-path={label.path ?? "edge_labeler"}>
            <div><strong>{edge.title ?? edge.work_id}</strong>{edge.year !== null && <span className="review-muted"> ({edge.year})</span>}</div>
            <div><span className="review-question">{edgeRelationLabel(label.relation)}</span> {label.reason}</div>
            {label.quote !== null && label.quote !== "" && <blockquote>“{label.quote}”</blockquote>}
            {label.passage !== null && label.passage !== "" && <details><summary>Citing passage{label.page !== null ? ` · p. ${label.page}` : ""}</summary><p>{label.passage}</p></details>}
            <div className="review-muted">{pathLabel(label.path)}{engagement !== null ? ` · ${engagement}` : ""} · {label.review_state}</div>
          </li>;
        })}</ol>}
      </>;
    }
    default:
      return null;
  } })();
  return <>
    <ComponentWarnings componentId={component.id} fields={fields} />
    {body}
  </>;
}

function anchorsFor(dimension: RubricDimension, rubric: Rubric): Record<string, string> {
  return dimension.anchors ?? (dimension.kind === "score" ? rubric.scale.anchors : {});
}

type DimensionRowProps = {
  row: FocusRow;
  rubric: Rubric;
  draft: ReviewDraft;
  focused: boolean;
  onFocus: () => void;
  onChange: (next: ReviewDraft) => void;
};

function DimensionRow({ row, rubric, draft, focused, onFocus, onChange }: DimensionRowProps) {
  const { dimension } = row;
  const value = valueOf(draft, row);
  const set = (next: boolean | number | string | null) => onChange(setRowValue(draft, row, next));
  let control: ReactNode;
  if (dimension.kind === "score") {
    const scores = Array.from({ length: rubric.scale.max - rubric.scale.min + 1 }, (_, index) => rubric.scale.min + index);
    const anchors = anchorsFor(dimension, rubric);
    control = <div className="review-choices">{scores.map((score) => (
      <button key={score} type="button" className={value === score ? "is-selected" : ""} title={anchors[String(score)] ?? ""} onClick={() => set(value === score ? null : score)}>{score}</button>
    ))}</div>;
  } else if (dimension.kind === "pass_fail") {
    control = <div className="review-choices">
      <button type="button" className={value === true ? "is-selected is-pass" : ""} onClick={() => set(value === true ? null : true)}>Pass <kbd>y</kbd></button>
      <button type="button" className={value === false ? "is-selected is-fail" : ""} onClick={() => set(value === false ? null : false)}>Fail <kbd>n</kbd></button>
    </div>;
  } else if (dimension.kind === "choice") {
    control = <div className="review-choices">{(dimension.options ?? []).map((option, index) => (
      <button key={option.id} type="button" className={value === option.id ? "is-selected" : ""} onClick={() => set(value === option.id ? null : option.id)}>
        <kbd>{index + 1}</kbd> {option.label}
      </button>
    ))}</div>;
  } else {
    const count = typeof value === "number" ? value : null;
    control = <div className="review-choices">
      <button type="button" aria-label="One fewer" onClick={() => set(Math.max(0, (count ?? 0) - 1))}>−</button>
      <input type="number" min={0} aria-label={dimension.label} value={count ?? ""} placeholder="–"
        onChange={(event) => set(event.target.value === "" ? null : Math.max(0, Math.floor(Number(event.target.value))))} />
      <button type="button" aria-label="One more" onClick={() => set((count ?? 0) + 1)}>+</button>
    </div>;
  }
  const anchors = anchorsFor(dimension, rubric);
  return (
    <div className={`review-dimension ${focused ? "is-focused" : ""}`} data-review-row={rowKey(row)} onMouseDown={onFocus}>
      <div className="review-dimension-head">
        <span className="review-dimension-label">{dimension.label}</span>
        <span className="review-dimension-question">{dimension.question}</span>
      </div>
      {control}
      {focused && Object.keys(anchors).length > 0 && <dl className="review-anchors">
        {Object.entries(anchors).map(([score, anchor]) => <div key={score}><dt>{score}</dt><dd>{anchor}</dd></div>)}
      </dl>}
    </div>
  );
}

type ItemVerdictsProps = {
  component: RubricComponent;
  fields: ReviewedFields;
  draft: ReviewDraft;
  onChange: (next: ReviewDraft) => void;
};

/** One click row per sampled citation label: ok, or its one main problem. */
function ItemVerdicts({ component, fields, draft, onChange }: ItemVerdictsProps) {
  const verdict = component.item_verdict;
  if (verdict === undefined) return null;
  const items = verdictItems(component, fields);
  if (items.length === 0) return null;
  const titles = new Map(readEdgeLabels(fields).sample.map((edge) => [edge.label?.claim_id ?? "", edge.title ?? edge.work_id]));
  return <div className="review-dimension review-verdicts">
    <div className="review-dimension-head">
      <span className="review-dimension-label">{verdict.label}</span>
      <span className="review-dimension-question">{verdict.question}</span>
    </div>
    {items.map((item, index) => {
      const current = verdictOf(draft, component, item);
      return <div key={item} className="review-verdict-row">
        <span className="review-muted">{index + 1}. {titles.get(item) ?? item}</span>
        <div className="review-choices">{verdict.options.map((option) => (
          <button key={option.id} type="button" className={current === option.id ? "is-selected" : ""}
            onClick={() => onChange(setVerdict(draft, component, item, current === option.id ? null : option.id))}>{option.label}</button>
        ))}</div>
      </div>;
    })}
  </div>;
}

type IssueFormState = { component: string; severity: string; tag: string; note: string; quote: string };

export function ReviewPanel({ paperId, onClose, onOpenPaper }: ReviewPanelProps) {
  const [rubric, setRubric] = useState<Rubric | null>(null);
  const [manifest, setManifest] = useState<Manifest | null>(null);
  const [progress, setProgress] = useState<ProgressRow[]>([]);
  const [paper, setPaper] = useState<PaperReview | null>(null);
  const [draft, setDraft] = useState<ReviewDraft>(emptyDraft);
  const [focusIndex, setFocusIndex] = useState(0);
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });
  const [loadError, setLoadError] = useState<string | null>(null);
  const [issueForm, setIssueForm] = useState<IssueFormState | null>(null);
  const [changes, setChanges] = useState(0);
  const draftRef = useRef<ReviewDraft>(emptyDraft());
  const pendingRef = useRef(false);
  // The citation labels on screen, sent with each save so a first save
  // cannot pin labels the reviewer never saw.
  const pinRef = useRef<EdgeLabelsPin | null>(null);
  const panelRef = useRef<HTMLElement>(null);
  const issueNoteRef = useRef<HTMLTextAreaElement>(null);
  const snapshotId = paper?.snapshot?.snapshot_id ?? null;

  const refreshProgress = useCallback(() => {
    void reviewApi.progress().then(setProgress).catch(() => {
      // Progress is a convenience; grading still works without it.
    });
  }, []);

  useEffect(() => {
    let cancelled = false;
    void Promise.all([reviewApi.rubric(), reviewApi.manifest()]).then(([nextRubric, nextManifest]) => {
      if (cancelled) return;
      setRubric(nextRubric);
      setManifest(nextManifest);
    }).catch((reason: unknown) => {
      if (!cancelled) setLoadError(reason instanceof Error ? reason.message : "Could not load the review rubric");
    });
    refreshProgress();
    panelRef.current?.focus();
    return () => { cancelled = true; };
  }, [refreshProgress]);

  useEffect(() => {
    if (paperId === null) return;
    let cancelled = false;
    void reviewApi.paper(paperId).then((next) => {
      if (cancelled) return;
      const nextDraft = draftFromReview(next.review);
      draftRef.current = nextDraft;
      setPaper(next);
      setDraft(nextDraft);
      if (next.review !== null) setSaveState({ kind: "saved", at: next.review.updated_at });
    }).catch((reason: unknown) => {
      if (!cancelled) setLoadError(reason instanceof Error ? reason.message : "Could not load this paper's review");
    });
    return () => { cancelled = true; };
  }, [paperId]);

  const save = useCallback(async (): Promise<void> => {
    if (paperId === null || snapshotId === null || !pendingRef.current) return;
    pendingRef.current = false;
    setSaveState({ kind: "saving" });
    try {
      const saved = await reviewApi.save(paperId, toInput(snapshotId, draftRef.current, pinRef.current));
      setSaveState({ kind: "saved", at: saved.updated_at });
      refreshProgress();
    } catch (reason: unknown) {
      if (reason instanceof ApiError && reason.status === 409) setSaveState({ kind: "stale", message: reason.message });
      else {
        pendingRef.current = true;
        setSaveState({ kind: "error", message: reason instanceof Error ? reason.message : "Could not save the review" });
      }
    }
  }, [paperId, refreshProgress, snapshotId]);

  useEffect(() => {
    if (changes === 0) return;
    const timer = window.setTimeout(() => { void save(); }, AUTOSAVE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [changes, save]);

  // Leaving the panel saves whatever the debounce has not sent yet.
  const saveRef = useRef(save);
  useEffect(() => { saveRef.current = save; }, [save]);
  useEffect(() => () => { void saveRef.current(); }, []);

  const change = useCallback((next: ReviewDraft): void => {
    draftRef.current = next;
    pendingRef.current = true;
    setDraft(next);
    setChanges((count) => count + 1);
  }, []);

  const rows = useMemo(() => (rubric === null ? [] : focusRows(rubric, draft)), [draft, rubric]);
  const focusedRow = rows[Math.min(focusIndex, Math.max(0, rows.length - 1))] ?? null;
  const manifestPaper = manifest?.papers.find((entry) => entry.id === paper?.manifest_id) ?? null;
  const fields = shownFields(paper?.snapshot ?? null, paper?.review ?? null);
  const done = rubric === null ? null : completion(rubric, draft, fields);
  const pin = edgeLabelsPin(fields);
  const pinKey = pin === null ? "" : JSON.stringify(pin);
  useEffect(() => { pinRef.current = pinKey === "" ? null : JSON.parse(pinKey) as EdgeLabelsPin; }, [pinKey]);
  const counts = progressCounts(progress);

  const focusRow = useCallback((index: number): void => {
    setFocusIndex(index);
    const row = rows[index];
    if (row === undefined) return;
    window.requestAnimationFrame(() => {
      panelRef.current?.querySelector(`[data-review-row="${rowKey(row)}"]`)?.scrollIntoView({ block: "nearest" });
    });
  }, [rows]);

  const openIssueForm = useCallback((component: string): void => {
    const selection = window.getSelection()?.toString().trim() ?? "";
    setIssueForm({ component, severity: rubric?.issues.severities[1]?.id ?? rubric?.issues.severities[0]?.id ?? "", tag: "", note: "", quote: selection });
    window.requestAnimationFrame(() => issueNoteRef.current?.focus());
  }, [rubric]);

  // The app's Escape/q chain asks before closing: close the issue form, or
  // leave a text field, first.
  useEffect(() => {
    const panel = panelRef.current;
    if (panel === null) return;
    const onQuit = (event: Event): void => {
      if (issueForm !== null) {
        event.preventDefault();
        setIssueForm(null);
        panel.focus();
      } else if (isEditable(document.activeElement) && panel.contains(document.activeElement)) {
        event.preventDefault();
        panel.focus();
      }
    };
    panel.addEventListener("review-quit", onQuit);
    return () => panel.removeEventListener("review-quit", onQuit);
  }, [issueForm]);

  const onKeyDown = (event: ReactKeyboardEvent<HTMLElement>): void => {
    if (rubric === null || isEditable(event.target) || event.ctrlKey || event.metaKey || event.altKey) return;
    const component = focusedRow === null || focusedRow.componentId === OVERALL ? null : rubric.components.find((entry) => entry.id === focusedRow.componentId) ?? null;
    let handled = true;
    if (event.key === "j" || event.key === "ArrowDown") focusRow(Math.min(rows.length - 1, focusIndex + 1));
    else if (event.key === "k" || event.key === "ArrowUp") focusRow(Math.max(0, focusIndex - 1));
    else if (event.key === "]" || event.key === "[") {
      const current = focusedRow?.componentId;
      const starts = rows.map((row, index) => ({ row, index })).filter(({ row, index }) => index === 0 || rows[index - 1]?.componentId !== row.componentId);
      const position = starts.findIndex(({ row }) => row.componentId === current);
      const target = starts[Math.max(0, Math.min(starts.length - 1, position + (event.key === "]" ? 1 : -1)))];
      if (target !== undefined) focusRow(target.index);
    } else if (event.key === "x" && component !== null) {
      change(updateGrade(draft, component.id, { not_produced: !gradeOf(draft, component.id).not_produced }));
    } else if (event.key === "r" && component?.hard_reject !== undefined) {
      change(updateGrade(draft, component.id, { hard_reject: !gradeOf(draft, component.id).hard_reject }));
    } else if (event.key === "i") openIssueForm(component?.id ?? GENERAL_COMPONENT);
    else if (event.key === "e" && focusedRow !== null) {
      panelRef.current?.querySelector<HTMLTextAreaElement>(`[data-review-notes="${focusedRow.componentId}"]`)?.focus();
    } else if (focusedRow !== null) {
      const next = applyGradingKey(draft, focusedRow, event.key, rubric.scale);
      if (next === null) handled = false;
      else change(next);
    } else handled = false;
    if (handled) {
      event.preventDefault();
      event.stopPropagation();
    }
  };

  const addIssue = (): void => {
    if (issueForm === null || (issueForm.note.trim() === "" && issueForm.quote.trim() === "")) return;
    const issue: IssueFlag = {
      id: nextIssueId(draft.issues),
      component: issueForm.component,
      severity: issueForm.severity,
      tag: issueForm.tag === "" ? null : issueForm.tag,
      note: issueForm.note.trim(),
      quote: issueForm.quote.trim() === "" ? null : issueForm.quote.trim(),
    };
    change({ ...draft, issues: [...draft.issues, issue] });
    setIssueForm(null);
    panelRef.current?.focus();
  };

  const saveLabel = saveState.kind === "saving" ? "Saving…"
    : saveState.kind === "saved" ? `Saved ${new Date(saveState.at).toLocaleTimeString()}`
    : saveState.kind === "error" ? `Not saved: ${saveState.message}`
    : saveState.kind === "stale" ? saveState.message
    : "Changes save automatically";

  return (
    <div className="review-backdrop" role="presentation" onMouseDown={onClose}>
      <section ref={panelRef} className="review-panel" role="dialog" aria-modal="true" aria-label="Review reader output" tabIndex={-1}
        onMouseDown={(event) => event.stopPropagation()} onKeyDown={onKeyDown}>
        <header>
          <div>
            <h2>Review{paper === null ? "" : ` · ${paper.paper_title}`}</h2>
            <p className="review-muted">
              {paper?.manifest_id != null ? <span className="review-badge">{paper.manifest_id}</span> : paperId !== null && <span className="review-badge is-muted">not in the evaluation set</span>}
              {paper?.snapshot != null && <> pipeline <code>{paper.snapshot.pipeline_version ?? "unversioned"}</code> · generated {paper.snapshot.generated_at === null ? "at an unknown time" : new Date(paper.snapshot.generated_at).toLocaleString()}{paper.snapshot.provider !== null && <> · {paper.snapshot.provider}</>}</>}
            </p>
          </div>
          <div className="review-header-status">
            {done !== null && paper?.snapshot != null && <span className={`review-meter ${done.done ? "is-done" : ""}`}>{done.graded}/{done.total} graded</span>}
            <span className={`review-save is-${saveState.kind}`} role="status">{saveLabel}</span>
            <button type="button" aria-label="Close review" onClick={onClose}>×</button>
          </div>
        </header>

        <nav className="review-progress" aria-label="Evaluation set progress">
          <span className="review-muted">{counts.inLibrary}/{counts.total} in library · {counts.analysed} analysed · {counts.reviewed} reviewed</span>
          <div className="review-progress-papers">
            {progress.map((row) => {
              const state = progressState(row);
              return <button key={row.manifest_id} type="button" className={`review-progress-paper is-${state} ${row.paper_id !== null && row.paper_id === paperId ? "is-current" : ""}`}
                disabled={row.paper_id === null} title={`${row.title} (${row.year ?? "n.d."}) — ${state.replace("-", " ")}${row.paper_id === null ? `; add ${row.filename} to the library` : ""}`}
                onClick={() => { if (row.paper_id !== null) onOpenPaper(row.paper_id); }}>
                {row.manifest_id}
              </button>;
            })}
          </div>
        </nav>

        {loadError !== null && <p className="review-error" role="alert">{loadError}</p>}
        {paperId === null ? <p className="review-empty">Open a paper to review it. Papers from the evaluation set are listed above; fetch them with <code>scripts/review/fetch_papers.py</code>.</p>
          : paper === null || rubric === null ? <p className="review-empty"><span className="loader" /> Loading…</p>
          : fields === null ? <p className="review-empty">Analyze this paper first; the review grades the reader’s output.</p>
          : <div className="review-body">
            <p className="review-purpose">{rubric.purpose} <span className="review-muted">About {rubric.time_budget_minutes} minutes. Keys: <kbd>j</kbd>/<kbd>k</kbd> move, <kbd>0</kbd>–<kbd>4</kbd> score, <kbd>y</kbd>/<kbd>n</kbd> pass or fail, <kbd>1</kbd>–<kbd>9</kbd> choose, <kbd>+</kbd>/<kbd>−</kbd> count, <kbd>x</kbd> not produced, <kbd>r</kbd> hard reject, <kbd>i</kbd> flag an issue, <kbd>e</kbd> notes, <kbd>[</kbd>/<kbd>]</kbd> component.</span></p>
            <GeneralWarnings fields={fields} />
            {rubric.components.map((component) => {
              const grade = gradeOf(draft, component.id);
              const produced = isProduced(component.id, fields);
              return <article key={component.id} className={`review-component ${component.priority === 1 ? "is-priority" : ""} ${done?.components[component.id] === true ? "is-complete" : ""}`}>
                <div className="review-output">
                  <h3>{component.label}{!produced && <span className="review-badge is-warning">not produced</span>}</h3>
                  <p className="review-question">{component.question}</p>
                  {component.guidance !== undefined && <p className="review-hint">{component.guidance}</p>}
                  <ComponentOutput component={component} fields={fields} manifestPaper={manifestPaper} />
                </div>
                <div className="review-grades">
                  <div className="review-flags">
                    <label><input type="checkbox" checked={grade.not_produced} onChange={(event) => change(updateGrade(draft, component.id, { not_produced: event.target.checked }))} /> Not produced <kbd>x</kbd></label>
                    {component.hard_reject !== undefined && <label className="is-reject" title={component.hard_reject.question}>
                      <input type="checkbox" checked={grade.hard_reject} onChange={(event) => change(updateGrade(draft, component.id, { hard_reject: event.target.checked }))} /> {component.hard_reject.label} <kbd>r</kbd>
                    </label>}
                  </div>
                  {component.item_verdict !== undefined && !grade.not_produced && <ItemVerdicts component={component} fields={fields} draft={draft} onChange={change} />}
                  {visibleDimensions(component, grade).map((dimension) => {
                    const row: FocusRow = { componentId: component.id, dimension };
                    const index = rows.findIndex((candidate) => rowKey(candidate) === rowKey(row));
                    return <DimensionRow key={dimension.id} row={row} rubric={rubric} draft={draft} focused={index === focusIndex} onFocus={() => setFocusIndex(index)} onChange={change} />;
                  })}
                  <textarea data-review-notes={component.id} rows={2} placeholder="Notes" value={grade.notes}
                    onChange={(event) => change(updateGrade(draft, component.id, { notes: event.target.value }))} />
                  <button type="button" className="review-link" onClick={() => openIssueForm(component.id)}>Flag an issue <kbd>i</kbd></button>
                </div>
              </article>;
            })}

            <article className="review-component">
              <div className="review-output">
                <h3>Overall</h3>
                <p className="review-question">{rubric.overall.question}</p>
              </div>
              <div className="review-grades">
                {(() => {
                  const row: FocusRow = { componentId: OVERALL, dimension: rubric.overall };
                  const index = rows.length - 1;
                  return <DimensionRow row={row} rubric={rubric} draft={draft} focused={index === focusIndex} onFocus={() => setFocusIndex(index)} onChange={change} />;
                })()}
                <textarea data-review-notes={OVERALL} rows={3} placeholder="Free notes on this paper’s output" value={draft.notes}
                  onChange={(event) => change({ ...draft, notes: event.target.value })} />
              </div>
            </article>

            <section className="review-issues" aria-label="Flagged issues">
              <h3>Flagged issues <span className="review-muted">({draft.issues.length})</span></h3>
              <p className="review-hint">{rubric.issues.purpose}</p>
              {draft.issues.length > 0 && <ul className="review-list">{draft.issues.map((issue) => <li key={issue.id} className={`review-issue is-${issue.severity}`}>
                <span className="review-badge">{issue.severity}</span> <strong>{rubric.components.find((entry) => entry.id === issue.component)?.label ?? issue.component}</strong>
                {issue.tag !== null && <span className="review-muted"> · {issue.tag}</span>}
                {issue.note !== "" && <div>{issue.note}</div>}
                {issue.quote !== null && <blockquote>{issue.quote}</blockquote>}
                <button type="button" className="review-link" onClick={() => change({ ...draft, issues: draft.issues.filter((entry) => entry.id !== issue.id) })}>Remove</button>
              </li>)}</ul>}
              {issueForm === null ? <button type="button" className="review-link" onClick={() => openIssueForm(focusedRow?.componentId === OVERALL || focusedRow === null ? GENERAL_COMPONENT : focusedRow.componentId)}>Flag an issue <kbd>i</kbd></button>
                : <div className="review-issue-form">
                  <label>Component
                    <select value={issueForm.component} onChange={(event) => setIssueForm({ ...issueForm, component: event.target.value })}>
                      {rubric.components.map((entry) => <option key={entry.id} value={entry.id}>{entry.label}</option>)}
                      <option value={GENERAL_COMPONENT}>General</option>
                    </select>
                  </label>
                  <label>Severity
                    <select value={issueForm.severity} onChange={(event) => setIssueForm({ ...issueForm, severity: event.target.value })}>
                      {rubric.issues.severities.map((entry) => <option key={entry.id} value={entry.id}>{entry.label}</option>)}
                    </select>
                  </label>
                  <label>Tag
                    <select value={issueForm.tag} onChange={(event) => setIssueForm({ ...issueForm, tag: event.target.value })}>
                      <option value="">(none)</option>
                      {rubric.issues.tags.map((tag) => <option key={tag} value={tag}>{tag.replaceAll("_", " ")}</option>)}
                    </select>
                  </label>
                  <label className="is-wide">What is wrong
                    <textarea ref={issueNoteRef} rows={2} value={issueForm.note} onChange={(event) => setIssueForm({ ...issueForm, note: event.target.value })}
                      onKeyDown={(event) => { if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) { event.preventDefault(); addIssue(); } }} />
                  </label>
                  <label className="is-wide">Quoted text (select text in the output before pressing <kbd>i</kbd> to fill this)
                    <textarea rows={2} value={issueForm.quote} onChange={(event) => setIssueForm({ ...issueForm, quote: event.target.value })} />
                  </label>
                  <div className="review-issue-actions">
                    <button type="button" onClick={addIssue} disabled={issueForm.note.trim() === "" && issueForm.quote.trim() === ""}>Add issue <kbd>⌘↵</kbd></button>
                    <button type="button" className="review-link" onClick={() => { setIssueForm(null); panelRef.current?.focus(); }}>Cancel</button>
                  </div>
                </div>}
            </section>

            {paper.earlier.length > 0 && <section className="review-earlier">
              <h3>Earlier reviews of replaced output</h3>
              <ul className="review-list">{paper.earlier.map((earlier) => <li key={earlier.snapshot_id}>
                <code>{earlier.pipeline_version ?? "unversioned"}</code> · output from {earlier.generated_at ?? "unknown time"} · reviewed {new Date(earlier.updated_at).toLocaleString()}
                {earlier.overall !== null && <> · overall {earlier.overall}</>}
              </li>)}</ul>
            </section>}
          </div>}
      </section>
    </div>
  );
}
