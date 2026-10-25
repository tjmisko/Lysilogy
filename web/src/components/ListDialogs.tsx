import { useEffect, useMemo, useRef, useState } from "react";

import { api } from "../lib/api";
import { claimFromText, dedupeHits, hitToAdd } from "../lib/listSearch";
import type { ListNodeAdd, Work, WorkCitations, WorkHit } from "../types";
import { DialogShell } from "./ListDialogShell";
import { dialogDirection, isTypingTarget, useDialogKeys } from "../lib/dialogKeys";

const SEARCH_DEBOUNCE_MS = 180;
const POLL_MILLISECONDS = 1000;
const CHECK_LIMIT = 100;

function failureMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : "The request failed.";
}

function workLine(work: Work): string {
  const authors = work.display_authors?.slice(0, 2).join(", ") ?? "";
  return [authors, work.display_year === null ? "" : String(work.display_year)].filter((part) => part !== "").join(" · ");
}

// ---- A generic list of choices -------------------------------------------------------------

export type Choice = { id: string; label: string; detail?: string; checked?: boolean };

/** Pick one choice (`↵`) or tick several (`space`, then `↵`). Used for topics, edges and context links. */
export function ChoiceDialog(props: {
  eyebrow: string;
  title: string;
  choices: readonly Choice[];
  multiple: boolean;
  emptyText: string;
  confirmLabel: string;
  onConfirm: (ids: string[]) => void;
  onClose: () => void;
}) {
  const { eyebrow, title, choices, multiple, emptyText, confirmLabel, onConfirm, onClose } = props;
  const [active, setActive] = useState(0);
  const [checked, setChecked] = useState<ReadonlySet<string>>(() => new Set(choices.filter((choice) => choice.checked === true).map((choice) => choice.id)));
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const last = Math.max(0, choices.length - 1);
  const at = Math.min(active, last);

  useEffect(() => {
    itemRefs.current[at]?.scrollIntoView({ block: "nearest" });
  }, [at]);

  useDialogKeys((event) => {
    if (event.key === "Escape" || (event.key === "q" && !isTypingTarget(event.target))) {
      onClose();
      return true;
    }
    const direction = dialogDirection(event);
    if (direction !== null) {
      setActive((index) => Math.max(0, Math.min(last, index + direction)));
      return true;
    }
    if (event.key === " " && multiple) {
      const choice = choices[at];
      if (choice === undefined) return true;
      setChecked((current) => {
        const next = new Set(current);
        if (next.has(choice.id)) next.delete(choice.id);
        else next.add(choice.id);
        return next;
      });
      return true;
    }
    if (event.key === "Enter") {
      if (multiple) onConfirm([...checked]);
      else if (choices[at] !== undefined) onConfirm([choices[at].id]);
      return true;
    }
    return false;
  });

  return (
    <DialogShell
      eyebrow={eyebrow}
      title={title}
      onClose={onClose}
      footer={<>
        <span><kbd>j/k</kbd> move</span>
        {multiple && <span><kbd>space</kbd> toggle</span>}
        <span><kbd>↵</kbd> {confirmLabel}</span>
        <span><kbd>esc</kbd> cancel</span>
      </>}
    >
      <div className="switcher-results" role="listbox" aria-multiselectable={multiple}>
        {choices.map((choice, index) => (
          <button
            key={choice.id}
            ref={(node) => { itemRefs.current[index] = node; }}
            type="button"
            role="option"
            aria-selected={multiple ? checked.has(choice.id) : index === at}
            className={index === at ? "is-active" : ""}
            tabIndex={-1}
            onMouseEnter={() => setActive(index)}
            onClick={() => {
              if (!multiple) onConfirm([choice.id]);
              else setChecked((current) => {
                const next = new Set(current);
                if (next.has(choice.id)) next.delete(choice.id);
                else next.add(choice.id);
                return next;
              });
            }}
          >
            <span className="list-choice-mark" aria-hidden="true">{multiple ? (checked.has(choice.id) ? "☑" : "☐") : ""}</span>
            <span><strong>{choice.label}</strong>{choice.detail !== undefined && <small>{choice.detail}</small>}</span>
          </button>
        ))}
        {choices.length === 0 && <p>{emptyText}</p>}
      </div>
    </DialogShell>
  );
}

// ---- Add by name (`a`) ---------------------------------------------------------------------

type AddRow = { kind: "hit"; hit: WorkHit } | { kind: "claim" };

function hitBadge(hit: WorkHit, onList: boolean): string {
  if (onList) return "on list";
  if (hit.source === "library") return "held";
  return hit.source === "provider" ? "provider" : "graph";
}

/** Search the library and graph (`Tab` adds the provider); no match falls back to a claim that gets verified. */
export function AddByNameDialog(props: {
  existingWorkIds: ReadonlySet<string>;
  onAdd: (add: ListNodeAdd) => void;
  onClose: () => void;
}) {
  const { existingWorkIds, onAdd, onClose } = props;
  const [query, setQuery] = useState("");
  const [provider, setProvider] = useState(false);
  const [hits, setHits] = useState<WorkHit[]>([]);
  const [searching, setSearching] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const text = query.trim();

  useEffect(() => { inputRef.current?.focus(); }, []);

  useEffect(() => {
    if (text === "") {
      window.queueMicrotask(() => { setHits([]); setSearching(false); setProblem(null); });
      return;
    }
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      setSearching(true);
      void api.searchWorks(text, provider, controller.signal).then((found) => {
        if (controller.signal.aborted) return;
        setHits(dedupeHits(found.results));
        setProblem(null);
      }).catch((reason: unknown) => {
        if (controller.signal.aborted) return;
        setHits([]);
        setProblem(failureMessage(reason));
      }).finally(() => {
        if (!controller.signal.aborted) setSearching(false);
      });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      controller.abort();
      window.clearTimeout(timer);
    };
  }, [provider, text]);

  const rows = useMemo<AddRow[]>(
    () => [...hits.map((hit): AddRow => ({ kind: "hit", hit })), ...(text === "" ? [] : [{ kind: "claim" } as const])],
    [hits, text],
  );
  const last = Math.max(0, rows.length - 1);
  const at = Math.min(active, last);

  useEffect(() => {
    itemRefs.current[at]?.scrollIntoView({ block: "nearest" });
  }, [at]);

  const choose = (row: AddRow | undefined): void => {
    if (row === undefined) return;
    if (row.kind === "claim") {
      const add = claimFromText(text);
      if (add !== null) onAdd(add);
      return;
    }
    onAdd(hitToAdd(row.hit));
  };

  useDialogKeys((event) => {
    if (event.key === "Escape") {
      onClose();
      return true;
    }
    if (event.key === "Tab") {
      setProvider((on) => !on);
      return true;
    }
    const direction = dialogDirection(event);
    if (direction !== null) {
      setActive((index) => Math.max(0, Math.min(last, index + direction)));
      return true;
    }
    if (event.key === "Enter") {
      choose(rows[at]);
      return true;
    }
    return false;
  });

  return (
    <DialogShell
      eyebrow="Add to this list"
      title="Add by name"
      hint="a"
      onClose={onClose}
      footer={<>
        <span><kbd>↑/↓</kbd> or <kbd>ctrl+n/p</kbd> move</span>
        <span><kbd>tab</kbd> provider search {provider ? "on" : "off"}</span>
        <span><kbd>↵</kbd> add</span>
        <span><kbd>esc</kbd> cancel</span>
      </>}
    >
      <label className="switcher-search">
        <span>›</span>
        <input
          ref={inputRef}
          value={query}
          maxLength={500}
          onChange={(event) => { setQuery(event.target.value); setActive(0); }}
          placeholder="Title or author"
          aria-label="Add a paper by name"
          autoComplete="off"
          spellCheck={false}
        />
      </label>
      <div className="switcher-results" role="listbox">
        {rows.map((row, index) => {
          const key = row.kind === "claim" ? "claim" : `${row.hit.source}:${row.hit.work_id ?? row.hit.paper_id ?? row.hit.openalex_id ?? row.hit.title}:${index}`;
          const onList = row.kind === "hit" && row.hit.work_id !== null && existingWorkIds.has(row.hit.work_id);
          return (
            <button
              key={key}
              ref={(node) => { itemRefs.current[index] = node; }}
              type="button"
              role="option"
              aria-selected={index === at}
              className={index === at ? "is-active" : ""}
              tabIndex={-1}
              onMouseEnter={() => setActive(index)}
              onClick={() => choose(row)}
            >
              <span className="list-choice-mark" aria-hidden="true">{row.kind === "claim" ? "?" : ""}</span>
              {row.kind === "claim"
                ? <span><strong>Add “{text}” as a claim</strong><small>It is checked against the providers before it counts as a paper.</small></span>
                : <span><strong>{row.hit.title}</strong><small>{[row.hit.authors.slice(0, 2).join(", "), row.hit.year === null ? "" : String(row.hit.year)].filter((part) => part !== "").join(" · ") || "No details"}</small></span>}
              {row.kind === "hit" && <em>{hitBadge(row.hit, onList)}</em>}
            </button>
          );
        })}
        {text === "" && <p>Type a title or an author.</p>}
        {text !== "" && searching && hits.length === 0 && <p>Searching…</p>}
        {problem !== null && <p role="alert">{problem}</p>}
      </div>
    </DialogShell>
  );
}

// ---- Add from references or cited-by (`A`) -------------------------------------------------

export type ReferenceDirection = "references" | "cited_by";

type Candidates = { work: Work; local: boolean };

function candidatesOf(citations: WorkCitations, direction: ReferenceDirection): Candidates[] {
  const works = direction === "references" ? citations.outgoing : citations.incoming;
  const seen = new Set<string>();
  const found: Candidates[] = [];
  for (const work of works) {
    if (seen.has(work.id) || work.id === citations.work.id) continue;
    seen.add(work.id);
    found.push({ work, local: (citations.local_papers[work.id]?.length ?? 0) > 0 });
  }
  return found;
}

/**
 * A checklist of the Work's references or cited-by (`Tab` switches). References are fetched on
 * first use when the graph does not know them yet. `↵` adds every ticked Work.
 */
export function ReferencesDialog(props: {
  workId: string;
  title: string;
  existingWorkIds: ReadonlySet<string>;
  onAdd: (workIds: string[], direction: ReferenceDirection) => void;
  onClose: () => void;
}) {
  const { workId, title, existingWorkIds, onAdd, onClose } = props;
  const [direction, setDirection] = useState<ReferenceDirection>("references");
  const [citations, setCitations] = useState<WorkCitations | null>(null);
  const [status, setStatus] = useState<string | null>("Loading…");
  const [checked, setChecked] = useState<ReadonlySet<string>>(() => new Set());
  const [active, setActive] = useState(0);
  const expanded = useRef(new Set<ReferenceDirection>());
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);

  useEffect(() => {
    const controller = new AbortController();
    const aborted = (): boolean => controller.signal.aborted;
    const wait = (): Promise<void> => new Promise((resolve) => window.setTimeout(resolve, POLL_MILLISECONDS));
    void (async () => {
      try {
        let current = await api.workCitations(workId, controller.signal);
        if (aborted()) return;
        setCitations(current);
        if (candidatesOf(current, direction).length === 0 && !expanded.current.has(direction)) {
          expanded.current.add(direction);
          setStatus(direction === "references" ? "Fetching references…" : "Fetching papers that cite it…");
          await api.expandWork(workId, { direction });
          for (let attempt = 0; attempt < 120 && !aborted(); attempt += 1) {
            await wait();
            const job = (await api.expansionStatus(workId, controller.signal, direction)).job;
            if (job.state !== "running") break;
          }
          if (aborted()) return;
          current = await api.workCitations(workId, controller.signal);
          if (aborted()) return;
          setCitations(current);
        }
        setStatus(null);
      } catch (reason: unknown) {
        if (!aborted()) setStatus(failureMessage(reason));
      }
    })();
    return () => controller.abort();
  }, [direction, workId]);

  const candidates = useMemo(() => (citations === null ? [] : candidatesOf(citations, direction)), [citations, direction]);
  const last = Math.max(0, candidates.length - 1);
  const at = Math.min(active, last);

  useEffect(() => {
    itemRefs.current[at]?.scrollIntoView({ block: "nearest" });
  }, [at]);

  const toggle = (id: string): void => {
    if (existingWorkIds.has(id)) return;
    setChecked((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else if (next.size < CHECK_LIMIT) next.add(id);
      return next;
    });
  };

  useDialogKeys((event) => {
    if (event.key === "Escape" || event.key === "q") {
      onClose();
      return true;
    }
    if (event.key === "Tab") {
      setDirection((now) => now === "references" ? "cited_by" : "references");
      setChecked(new Set());
      setActive(0);
      setStatus("Loading…");
      return true;
    }
    const move = dialogDirection(event);
    if (move !== null) {
      setActive((index) => Math.max(0, Math.min(last, index + move)));
      return true;
    }
    if (event.key === " ") {
      const candidate = candidates[at];
      if (candidate !== undefined) toggle(candidate.work.id);
      return true;
    }
    if (event.key === "Enter") {
      const chosen = checked.size > 0 ? [...checked] : candidates[at] !== undefined && !existingWorkIds.has(candidates[at].work.id) ? [candidates[at].work.id] : [];
      if (chosen.length === 0) return true;
      onAdd(chosen, direction);
      return true;
    }
    return false;
  });

  return (
    <DialogShell
      eyebrow={direction === "references" ? "References of" : "Cited by"}
      title={title}
      hint="A"
      onClose={onClose}
      footer={<>
        <span><kbd>j/k</kbd> move</span>
        <span><kbd>space</kbd> tick</span>
        <span><kbd>tab</kbd> {direction === "references" ? "cited-by" : "references"}</span>
        <span><kbd>↵</kbd> add {checked.size > 0 ? checked.size : "this one"}</span>
        <span><kbd>esc</kbd> cancel</span>
      </>}
    >
      <div className="switcher-results" role="listbox" aria-multiselectable="true">
        {candidates.map(({ work, local }, index) => {
          const onList = existingWorkIds.has(work.id);
          return (
            <button
              key={work.id}
              ref={(node) => { itemRefs.current[index] = node; }}
              type="button"
              role="option"
              aria-selected={checked.has(work.id)}
              aria-disabled={onList}
              className={`${index === at ? "is-active" : ""} ${onList ? "is-muted" : ""}`}
              tabIndex={-1}
              onMouseEnter={() => setActive(index)}
              onClick={() => toggle(work.id)}
            >
              <span className="list-choice-mark" aria-hidden="true">{onList ? "·" : checked.has(work.id) ? "☑" : "☐"}</span>
              <span>
                <strong>{work.display_title ?? "Untitled"}</strong>
                <small>{workLine(work) || "No details"}</small>
              </span>
              <em>{onList ? "on list" : local ? "held" : ""}</em>
            </button>
          );
        })}
        {status !== null && <p role="status">{status}</p>}
        {status === null && candidates.length === 0 && <p>{direction === "references" ? "No references are known for this paper." : "No papers citing it are known."}</p>}
      </div>
    </DialogShell>
  );
}
