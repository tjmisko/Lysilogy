import { useEffect, useRef, useState } from "react";
import { cachedPdfPreview, loadPdfPreview } from "../lib/pdfPreview";

export function PaperPreview({ url, title }: { url: string; title: string }) {
  const host = useRef<HTMLSpanElement>(null);
  const [near, setNear] = useState(false);
  const [image, setImage] = useState(() => cachedPdfPreview(url));
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const node = host.current;
    if (node === null) return;
    const observer = new IntersectionObserver(([entry]) => {
      setNear(entry?.isIntersecting ?? false);
    }, { rootMargin: "160px" });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!near || image !== undefined) return;
    const controller = new AbortController();
    void loadPdfPreview(url, controller.signal).then((preview) => {
      if (!controller.signal.aborted) setImage(preview);
    }).catch(() => {
      if (!controller.signal.aborted) setFailed(true);
    });
    return () => controller.abort();
  }, [image, near, url]);

  return <span ref={host} className="paper-preview" data-preview-state={image !== undefined ? "ready" : failed ? "failed" : "loading"}>
    {image !== undefined
      ? <img src={image} alt={`First page of ${title}`} draggable={false} />
      : <span className="paper-preview-placeholder" aria-hidden="true">{failed ? "Preview unavailable" : "Loading preview…"}</span>}
  </span>;
}
