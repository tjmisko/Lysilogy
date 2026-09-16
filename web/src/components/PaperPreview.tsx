import { useEffect, useRef, useState } from "react";
import { cachedPdfPreview, loadPdfPreview } from "../lib/pdfPreview";

export function PaperPreview({ url, title }: { url: string; title: string }) {
  const host = useRef<HTMLSpanElement>(null);
  const [near, setNear] = useState(false);
  const [preview, setPreview] = useState(() => ({ url, image: cachedPdfPreview(url), failed: false }));
  const image = preview.url === url ? preview.image : cachedPdfPreview(url);
  const failed = preview.url === url && preview.failed;

  useEffect(() => {
    const node = host.current;
    if (node === null) return;
    const observer = new IntersectionObserver(([entry]) => {
      if (!entry?.isIntersecting) return;
      setNear(true);
      observer.disconnect();
    }, { root: node.closest(".main-stage"), rootMargin: "1200px 0px" });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!near) return;
    let disposed = false;
    void loadPdfPreview(url, () => {
      const node = host.current;
      if (disposed || node === null) return Infinity;
      const { top, bottom } = node.getBoundingClientRect();
      const viewport = node.closest(".main-stage")?.getBoundingClientRect();
      return Math.max(0, top - (viewport?.bottom ?? window.innerHeight), (viewport?.top ?? 0) - bottom);
    }).then((image) => {
      if (!disposed) setPreview({ url, image, failed: false });
    }).catch(() => {
      if (!disposed) setPreview({ url, image: undefined, failed: true });
    });
    return () => { disposed = true; };
  }, [near, url]);

  return <span ref={host} className="paper-preview" data-preview-state={image !== undefined ? "ready" : failed ? "failed" : "loading"}>
    {image !== undefined
      ? <img src={image} alt={`First page of ${title}`} draggable={false} />
      : <span className="paper-preview-placeholder" aria-hidden="true">{failed ? "Preview unavailable" : "Loading preview…"}</span>}
  </span>;
}
