import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type SyntheticEvent,
} from "react";

import type {
  AnalysisProvider,
  Claim,
  Clarification,
  PaperSection,
} from "../types";

type DigestPanelProps = {
  section: PaperSection;
  claims: Claim[];
  onClose: () => void;
  onGloss: () => void;
  onOpenPage: (page: number) => void;
  onClarify: (
    selection: string,
    question: string,
    provider: AnalysisProvider,
  ) => Promise<Clarification>;
};

type Fragment = {
  id: string;
  type: "digest" | "quote" | "explanation" | "claim";
  label: string;
  text: string;
  page?: number;
};

function digestParagraphs(digest: string): string[] {
  const explicit = digest
    .split(/\n{2,}/u)
    .map((part) => part.trim())
    .filter(Boolean);
  if (explicit.length > 1) return explicit;
  const sentences = digest.match(/[^.!?]+(?:[.!?]+|$)/gu) ?? [digest];
  const grouped: string[] = [];
  for (let index = 0; index < sentences.length; index += 2) {
    grouped.push(sentences.slice(index, index + 2).join(" ").trim());
  }
  return grouped.filter(Boolean);
}

function isEditable(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;
}

export function DigestPanel({
  section,
  claims,
  onClose,
  onGloss,
  onOpenPage,
  onClarify,
}: DigestPanelProps) {
  const fragments = useMemo<Fragment[]>(() => {
    const digest = digestParagraphs(section.digest).map((text, index) => ({
      id: `digest-${index}`,
      type: "digest" as const,
      label: index === 0 ? "Contextual digest" : "Continued",
      text,
    }));
    const quotes = section.key_quotes.flatMap<Fragment>((quote, index) => [
      {
        id: `quote-${index}`,
        type: "quote",
        label: `${quote.significance.replace("_", " ")} · p. ${quote.page}`,
        text: quote.text,
        page: quote.page,
      },
      {
        id: `explanation-${index}`,
        type: "explanation",
        label: "Why this matters",
        text: quote.explanation,
        page: quote.page,
      },
    ]);
    const relatedClaims = claims
      .filter((claim) => claim.section_ids.includes(section.id))
      .map<Fragment>((claim, index) => ({
        id: `claim-${index}`,
        type: "claim",
        label: `${claim.strength} claim`,
        text: `${claim.statement} ${claim.support}`,
      }));
    return [...digest, ...quotes, ...relatedClaims];
  }, [claims, section]);

  const [active, setActive] = useState(0);
  const [visualAnchor, setVisualAnchor] = useState<number | null>(null);
  const [visualCursor, setVisualCursor] = useState(0);
  const [nativeSelection, setNativeSelection] = useState("");
  const [clarifySelection, setClarifySelection] = useState("");
  const [question, setQuestion] = useState("");
  const [provider, setProvider] = useState<AnalysisProvider>("heuristic");
  const [clarification, setClarification] = useState<Clarification | null>(null);
  const [clarifyError, setClarifyError] = useState<string | null>(null);
  const [clarifying, setClarifying] = useState(false);
  const [copied, setCopied] = useState(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const fragmentRefs = useRef<Array<HTMLElement | null>>([]);
  const questionRef = useRef<HTMLTextAreaElement>(null);

  const visualBounds = useMemo(
    () =>
      visualAnchor === null
        ? null
        : ([Math.min(visualAnchor, visualCursor), Math.max(visualAnchor, visualCursor)] as const),
    [visualAnchor, visualCursor],
  );

  const visualText = useCallback((): string => {
    if (visualBounds === null) return "";
    return fragments
      .slice(visualBounds[0], visualBounds[1] + 1)
      .map((fragment) => fragment.text)
      .join("\n\n");
  }, [fragments, visualBounds]);

  const beginClarification = useCallback((): void => {
    const selection =
      nativeSelection.trim() || visualText().trim() || (fragments[active]?.text ?? "");
    if (selection.length === 0) return;
    setClarifySelection(selection);
    setClarification(null);
    setClarifyError(null);
    window.setTimeout(() => questionRef.current?.focus(), 0);
  }, [active, fragments, nativeSelection, visualText]);

  useEffect(() => {
    fragmentRefs.current[active]?.focus({ preventScroll: true });
    fragmentRefs.current[active]?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [active]);

  useEffect(() => {
    const move = (delta: number): void => {
      const next = Math.max(0, Math.min(fragments.length - 1, active + delta));
      setActive(next);
      if (visualAnchor !== null) setVisualCursor(next);
    };
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        if (clarifySelection.length > 0 || clarification !== null) {
          setClarifySelection("");
          setClarification(null);
          setClarifyError(null);
        } else if (visualAnchor !== null) {
          setVisualAnchor(null);
        } else {
          onClose();
        }
        return;
      }
      if (isEditable(event.target) || event.metaKey || event.ctrlKey || event.altKey) return;
      switch (event.key) {
        case "j":
        case "l":
          event.preventDefault();
          move(1);
          break;
        case "k":
        case "h":
          event.preventDefault();
          move(-1);
          break;
        case "G":
          event.preventDefault();
          setActive(Math.max(0, fragments.length - 1));
          if (visualAnchor !== null) setVisualCursor(Math.max(0, fragments.length - 1));
          break;
        case "v":
          event.preventDefault();
          if (visualAnchor === null) {
            setVisualAnchor(active);
            setVisualCursor(active);
            window.getSelection()?.removeAllRanges();
            setNativeSelection("");
          } else {
            setVisualAnchor(null);
          }
          break;
        case "o":
          if (visualAnchor !== null) {
            event.preventDefault();
            const previousAnchor = visualAnchor;
            setVisualAnchor(visualCursor);
            setVisualCursor(previousAnchor);
            setActive(previousAnchor);
          }
          break;
        case "c":
          event.preventDefault();
          beginClarification();
          break;
        case "y": {
          event.preventDefault();
          const selection = nativeSelection.trim() || visualText().trim() || fragments[active]?.text;
          if (selection !== undefined && selection.length > 0) {
            void navigator.clipboard.writeText(selection).then(() => {
              setCopied(true);
              window.setTimeout(() => setCopied(false), 1_200);
            });
          }
          break;
        }
        case "g":
          event.preventDefault();
          onGloss();
          break;
        case "p":
          event.preventDefault();
          onOpenPage(section.pages.start);
          break;
        case "Enter": {
          const page = fragments[active]?.page;
          if (page !== undefined) {
            event.preventDefault();
            onOpenPage(page);
          }
          break;
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    active,
    beginClarification,
    clarification,
    clarifySelection,
    fragments,
    nativeSelection,
    onClose,
    onGloss,
    onOpenPage,
    section.pages.start,
    visualAnchor,
    visualCursor,
    visualText,
  ]);

  const captureNativeSelection = (): void => {
    const selection = window.getSelection();
    if (selection === null || selection.isCollapsed || selection.rangeCount === 0) {
      setNativeSelection("");
      return;
    }
    const range = selection.getRangeAt(0);
    if (contentRef.current?.contains(range.commonAncestorContainer) === true) {
      setNativeSelection(selection.toString().trim());
      setVisualAnchor(null);
    }
  };

  const submitClarification = async (event: SyntheticEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault();
    if (clarifySelection.length === 0 || clarifying) return;
    setClarifying(true);
    setClarifyError(null);
    try {
      setClarification(await onClarify(clarifySelection, question, provider));
    } catch (error) {
      setClarifyError(error instanceof Error ? error.message : "Clarification failed");
    } finally {
      setClarifying(false);
    }
  };

  return (
    <aside className="context-panel digest-panel" aria-label={`Digest: ${section.title}`}>
      <header className="panel-header">
        <div>
          <span className="eyebrow">Contextual digest</span>
          <h2>{section.title}</h2>
          <button className="page-jump" type="button" onClick={() => onOpenPage(section.pages.start)}>
            pages {section.pages.start}–{section.pages.end} ↗
          </button>
        </div>
        <button className="icon-button" type="button" onClick={onClose} aria-label="Close digest">
          ×
        </button>
      </header>

      <div className="selection-instructions" data-visual={visualAnchor !== null}>
        {visualAnchor === null ? (
          <><kbd>v</kbd> select passages with the keyboard · native selection also works</>
        ) : (
          <><strong>VISUAL</strong> <kbd>j/k</kbd> extend · <kbd>o</kbd> swap end · <kbd>c</kbd> clarify</>
        )}
      </div>

      <div className="digest-scroll" ref={contentRef} onMouseUp={captureNativeSelection}>
        {fragments.map((fragment, index) => {
          const fragmentPage = fragment.page;
          const selected =
            visualBounds !== null && index >= visualBounds[0] && index <= visualBounds[1];
          const className = [
            "digest-fragment",
            `fragment-${fragment.type}`,
            index === active ? "is-active" : "",
            selected ? "is-visual-selected" : "",
          ]
            .filter(Boolean)
            .join(" ");
          const Component = fragment.type === "quote" ? "blockquote" : "section";
          return (
            <Component
              key={fragment.id}
              ref={(node) => {
                fragmentRefs.current[index] = node;
              }}
              className={className}
              tabIndex={index === active ? 0 : -1}
              onFocus={() => setActive(index)}
              onClick={() => setActive(index)}
            >
              <span>{fragment.label}</span>
              <p>{fragment.text}</p>
              {fragmentPage !== undefined && (
                <button type="button" onClick={() => onOpenPage(fragmentPage)}>
                  source · p. {fragmentPage}
                </button>
              )}
            </Component>
          );
        })}
      </div>

      {(nativeSelection.length > 0 || visualAnchor !== null) && clarifySelection.length === 0 && (
        <div className="selection-actions">
          <span>{nativeSelection.length > 0 ? "Text selected" : "Visual passage selected"}</span>
          <button type="button" onClick={beginClarification}>
            Clarify <kbd>c</kbd>
          </button>
        </div>
      )}

      {clarifySelection.length > 0 && (
        <form className="clarify-composer" onSubmit={(event) => void submitClarification(event)}>
          <div className="selected-excerpt">“{clarifySelection}”</div>
          <label>
            What is unclear? <span>optional</span>
            <textarea
              ref={questionRef}
              value={question}
              onChange={(event) => setQuestion(event.target.value)}
              placeholder="What assumption is doing the work here?"
              rows={2}
              onKeyDown={(event) => {
                if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
                  event.currentTarget.form?.requestSubmit();
                }
              }}
            />
          </label>
          <div className="clarify-controls">
            <select
              value={provider}
              onChange={(event) => setProvider(event.target.value as AnalysisProvider)}
              aria-label="Clarification provider"
            >
              <option value="heuristic">Instant · offline</option>
              <option value="codex">Codex · local CLI</option>
              <option value="claude">Claude · local CLI</option>
            </select>
            <button type="submit" disabled={clarifying}>
              {clarifying ? "Reading context…" : "Explain"} <kbd>⌘↵</kbd>
            </button>
          </div>
          {clarifyError !== null && <p className="inline-error">{clarifyError}</p>}
          {clarification !== null && (
            <article className="clarification-result">
              <span className="eyebrow">Clarification · {clarification.provider}</span>
              <p>{clarification.answer}</p>
              {clarification.concepts.map((concept) => (
                <dl key={concept.term}>
                  <dt>{concept.term}</dt>
                  <dd>{concept.plain_language}</dd>
                </dl>
              ))}
              {clarification.limitation !== null && <small>{clarification.limitation}</small>}
            </article>
          )}
        </form>
      )}

      <footer className="panel-footer">
        <button type="button" onClick={onGloss}>Gloss <kbd>g</kbd></button>
        <span>{copied ? "Copied" : `${active + 1} / ${fragments.length}`}</span>
      </footer>
    </aside>
  );
}
