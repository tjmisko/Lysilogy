import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api } from "../lib/api";

type MarkdownReaderProps = {
  paperId: string;
  title: string;
  onConverted: () => void;
  onOpenPage: (page: number) => void;
};

type MarkdownBlock =
  | { kind: "heading"; level: number; text: string }
  | { kind: "paragraph"; lines: string[] }
  | { kind: "quote"; lines: string[] }
  | { kind: "list"; items: string[] }
  | { kind: "code"; text: string }
  | { kind: "rule" };

function isBlockStart(line: string): boolean {
  const trimmed = line.trim();
  return (
    /^#{1,6}\s/u.test(trimmed) ||
    /^>\s?/u.test(trimmed) ||
    /^[-*]\s+/u.test(trimmed) ||
    /^```/u.test(trimmed) ||
    /^-{3,}$/u.test(trimmed)
  );
}

function parseMarkdown(source: string): MarkdownBlock[] {
  const lines = source.replaceAll("\r\n", "\n").split("\n");
  const blocks: MarkdownBlock[] = [];
  let index = 0;
  while (index < lines.length) {
    const line = lines[index] ?? "";
    if (line.trim().length === 0) {
      index += 1;
      continue;
    }
    const heading = /^(#{1,6})\s+(.+)$/u.exec(line.trim());
    if (heading !== null) {
      blocks.push({ kind: "heading", level: heading[1]?.length ?? 1, text: heading[2] ?? "" });
      index += 1;
      continue;
    }
    if (/^-{3,}$/u.test(line.trim())) {
      blocks.push({ kind: "rule" });
      index += 1;
      continue;
    }
    if (line.trimStart().startsWith("```")) {
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !lines[index]?.trimStart().startsWith("```")) {
        code.push(lines[index] ?? "");
        index += 1;
      }
      blocks.push({ kind: "code", text: code.join("\n") });
      index += 1;
      continue;
    }
    if (/^>\s?/u.test(line.trimStart())) {
      const quote: string[] = [];
      while (index < lines.length && /^>\s?/u.test(lines[index]?.trimStart() ?? "")) {
        quote.push((lines[index] ?? "").trimStart().replace(/^>\s?/u, ""));
        index += 1;
      }
      blocks.push({ kind: "quote", lines: quote });
      continue;
    }
    if (/^[-*]\s+/u.test(line.trimStart())) {
      const items: string[] = [];
      while (index < lines.length && /^[-*]\s+/u.test(lines[index]?.trimStart() ?? "")) {
        items.push((lines[index] ?? "").trimStart().replace(/^[-*]\s+/u, ""));
        index += 1;
      }
      blocks.push({ kind: "list", items });
      continue;
    }

    const paragraph: string[] = [];
    while (
      index < lines.length &&
      (lines[index]?.trim().length ?? 0) > 0 &&
      !isBlockStart(lines[index] ?? "")
    ) {
      paragraph.push(lines[index] ?? "");
      index += 1;
    }
    blocks.push({ kind: "paragraph", lines: paragraph });
  }
  return blocks;
}

function decodeText(value: string): string {
  return value
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&amp;", "&")
    .replace(/\\([\\*_#[\]<>])/gu, "$1");
}

function inline(value: string): ReactNode[] {
  return value.split(/(`[^`]+`|\*\*[^*]+\*\*)/gu).map((part, index) => {
    const key = `${index}-${part.slice(0, 12)}`;
    if (part.startsWith("**") && part.endsWith("**")) {
      return <strong key={key}>{decodeText(part.slice(2, -2))}</strong>;
    }
    if (part.startsWith("`") && part.endsWith("`")) {
      return <code key={key}>{decodeText(part.slice(1, -1))}</code>;
    }
    return decodeText(part);
  });
}

function paragraph(lines: string[]): ReactNode {
  return lines.map((line, index) => (
    <span key={`${index}-${line.slice(0, 12)}`}>
      {inline(line.trimEnd())}
      {index + 1 < lines.length && (line.endsWith("  ") ? <br /> : " ")}
    </span>
  ));
}

export function MarkdownReader({
  paperId,
  title,
  onConverted,
  onOpenPage,
}: MarkdownReaderProps) {
  const [source, setSource] = useState<string | null>(null);
  const [raw, setRaw] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const blocks = useMemo(() => (source === null ? [] : parseMarkdown(source)), [source]);

  useEffect(() => {
    const controller = new AbortController();
    void api
      .markdown(paperId, controller.signal)
      .then((markdown) => {
        setSource(markdown);
        onConverted();
      })
      .catch((reason: unknown) => {
        if (!controller.signal.aborted) {
          setError(reason instanceof Error ? reason.message : "Could not convert the PDF");
        }
      });
    return () => controller.abort();
  }, [onConverted, paperId]);

  const copy = (): void => {
    if (source === null) return;
    void navigator.clipboard.writeText(source).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1_200);
    });
  };

  return (
    <section className="markdown-reader" aria-label={`Markdown: ${title}`} aria-busy={source === null}>
      <div className="markdown-toolbar">
        <div>
          <span className="eyebrow">Reconstructed text</span>
          <strong>{title}</strong>
        </div>
        <div className="markdown-controls">
          <div className="mini-switch" role="group" aria-label="Markdown display">
            <button type="button" className={!raw ? "is-active" : ""} onClick={() => setRaw(false)}>
              Read
            </button>
            <button type="button" className={raw ? "is-active" : ""} onClick={() => setRaw(true)}>
              Source
            </button>
          </div>
          <button type="button" onClick={copy} disabled={source === null}>
            {copied ? "Copied" : "Copy .md"}
          </button>
        </div>
      </div>

      {source === null && error === null && (
        <div className="reader-message markdown-loading">
          <span className="loader" /> Reconstructing Markdown from the PDF text layer…
        </div>
      )}
      {error !== null && <div className="reader-message error-message">{error}</div>}
      {source !== null && raw && <pre className="markdown-source">{source}</pre>}
      {source !== null && !raw && (
        <article className="markdown-document">
          {blocks.map((block, index) => {
            const key = `${index}-${block.kind}`;
            if (block.kind === "rule") return <hr key={key} />;
            if (block.kind === "code") return <pre key={key}><code>{block.text}</code></pre>;
            if (block.kind === "quote") return <blockquote key={key}>{paragraph(block.lines)}</blockquote>;
            if (block.kind === "list") {
              return <ul key={key}>{block.items.map((item) => <li key={item}>{inline(item)}</li>)}</ul>;
            }
            if (block.kind === "paragraph") return <p key={key}>{paragraph(block.lines)}</p>;

            const page = /^PDF page (\d+)$/iu.exec(block.text)?.[1];
            if (page !== undefined) {
              return (
                <button
                  key={key}
                  className="markdown-page-marker"
                  type="button"
                  onClick={() => onOpenPage(Number(page))}
                >
                  PDF page {page} <span>open source ↗</span>
                </button>
              );
            }
            const Heading = `h${Math.min(6, Math.max(1, block.level))}` as "h1";
            return <Heading key={key}>{inline(block.text)}</Heading>;
          })}
        </article>
      )}
    </section>
  );
}
