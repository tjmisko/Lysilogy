import { selectionSpans, selectionText, tokensInSpan, type ReadingIndex, type ReadingToken, type SourceSelection, type TextSpan } from './readingIndex.ts';
import { sourceCursor, sourceGrapheme } from './sourceMotions.ts';
import { alignLineGutters } from './lineGutters.ts';
import type { TextRect } from '../types';

export type CursorBlock = SourceSelection & { id: string; label: string; caption: string; page: number; rect: TextRect; kind: 'figure' | 'table' };
export type CursorSegment = TextSpan & { source: SourceSelection; token: ReadingToken; block?: CursorBlock };
export type CursorLine = TextSpan & { number: number; page: number; rect: TextRect; gutter?: number; block?: CursorBlock };
export type CursorDocument = { index: ReadingIndex; source: ReadingIndex; segments: CursorSegment[]; lines: CursorLine[]; blocks: CursorBlock[] };
const documents = new WeakMap<ReadingIndex, CursorDocument>();
const contains = (span: TextSpan, offset: number) => span.start <= offset && offset < span.end;
const overlaps = (a: TextSpan, b: TextSpan) => a.start < b.end && b.start < a.end;
const union = (a: TextRect, b: TextRect): TextRect => ({ x_min: Math.min(a.x_min,b.x_min), x_max: Math.max(a.x_max,b.x_max), y_min: Math.min(a.y_min,b.y_min), y_max: Math.max(a.y_max,b.y_max) });

function compact(source: ReadingIndex, pieces: TextSpan[]): SourceSelection {
  const spans: TextSpan[] = [];
  for (const piece of pieces.sort((a,b)=>a.start-b.start)) {
    const last=spans.at(-1);
    if (last !== undefined && (last.end >= piece.start || /^\s*$/u.test(source.text.slice(last.end,piece.start)))) last.end=Math.max(last.end,piece.end);
    else spans.push({...piece});
  }
  const bounds={start:spans[0]?.start??0,end:spans.at(-1)?.end??0};
  return spans.length>1?{...bounds,spans}:bounds;
}

/** A separate navigation projection; the searchable/source-coordinate index is immutable. */
export function cursorDocument(source: ReadingIndex): CursorDocument {
  const cached=documents.get(source); if(cached!==undefined)return cached;
  const body=source.objects.paragraph.filter(p=>p.kind==='body').flatMap(selectionSpans);
  const blocks: CursorBlock[]=[];
  for(const figure of source.figures) {
    const captionTokens=tokensInSpan(source,{start:figure.start,end:figure.end});
    const caption=captionTokens.flatMap(t=>t.rects).reduce<TextRect|null>((box,rect)=>box===null?rect:union(box,rect),null);
    if(caption===null)continue;
    const rect=figure.rect??caption;
    const members=figure.spans?.length ? tokensInSpan(source,figure) : source.tokens.filter(token=>token.page===figure.page && (overlaps(token,figure)
      || !body.some(span=>overlaps(span,token)) && token.rects.every(box=>box.x_min>=rect.x_min-1&&box.x_max<=rect.x_max+1&&box.y_min>=rect.y_min-1&&box.y_max<=rect.y_max+1)));
    if(members.length===0)continue;
    blocks.push({...compact(source,members),id:figure.id,label:figure.label,caption:figure.caption,page:figure.page,rect,
      kind:figure.kind==='table'||/^table\b/iu.test(figure.label)?'table':'figure'});
  }
  const membership=new Map<ReadingToken,CursorBlock>();
  for(const block of blocks)for(const token of tokensInSpan(source,block))if(!membership.has(token))membership.set(token,block);
  const index: ReadingIndex={...source,text:'',tokens:[],objects:{word:[],WORD:[],sentence:[],paragraph:[]},figures:[]};
  const result: CursorDocument={source,index,segments:[],lines:[],blocks};
  const emitted=new Set<CursorBlock>();
  for(const original of source.tokens) {
    const block=membership.get(original);
    if(block!==undefined&&emitted.has(block))continue;
    if(block!==undefined)emitted.add(block);
    const token=block===undefined?original:{...original,text:'\uFFFC',rects:[block.rect],atomic:true};
    const rect=token.rects[0];if(rect===undefined)continue;
    const previous=result.segments.at(-1),line=result.lines.at(-1);
    const before=previous?.token.rects[0];
    const sameLine=block===undefined&&previous?.block===undefined&&line?.page===token.page&&before!==undefined
      && Math.abs((rect.y_min+rect.y_max-before.y_min-before.y_max)/2)<Math.max(rect.y_max-rect.y_min,before.y_max-before.y_min)*.65
      && rect.x_min>=before.x_min-2&&rect.x_min-before.x_max<Math.max(30,(source.pages.find(p=>p.number===token.page)?.width??612)*.12)
      && !/[\r\n]/u.test(source.text.slice(previous?.source.end ?? original.start,original.start));
    if (index.text.length && !sameLine) index.text += '\n';
    if (sameLine && previous !== undefined && previous.source.end < original.start) {
      const gap = source.text.slice(previous.source.end, original.start);
      const start = index.text.length;
      index.text += gap;
      const space: ReadingToken = { ...original, start, end: index.text.length, text: gap,
        rects: [{ ...rect, x_min: before.x_max, x_max: rect.x_min }] };
      index.tokens.push(space);
      result.segments.push({ start, end: index.text.length, source: {start: previous.source.end, end: original.start}, token: space });
    }
    const start=index.text.length;index.text+=token.text;
    const item={start,end:index.text.length};
    const projected={...token,...item};index.tokens.push(projected);
    result.segments.push({...item,source:block??{start:original.start,end:original.end},token:projected,block});
    if(sameLine){line.end=item.end;line.rect=union(line.rect,rect);}
    else result.lines.push({...item,number:result.lines.length+1,page:token.page,rect,block});
  }
  const ranges=(pattern:RegExp)=>Array.from(index.text.matchAll(pattern),match=>({start:match.index,end:match.index+match[0].length}));
  index.objects.word=ranges(/[\p{L}\p{M}\p{N}_]+|[^\s\p{L}\p{M}\p{N}_\uFFFC]+|\uFFFC/gu);
  index.objects.WORD=ranges(/[^\s\uFFFC]+|\uFFFC/gu);
  index.objects.sentence=Array.from(new Intl.Segmenter('en',{granularity:'sentence'}).segment(index.text),part=>({start:part.index,end:part.index+part.segment.trimEnd().length}));
  index.pages=source.pages.map(page=>{
    const tokens=index.tokens.filter(token=>token.page===page.number);
    return {...page,start:tokens[0]?.start??0,end:tokens.at(-1)?.end??0};
  });
  alignLineGutters(result.lines);
  documents.set(source,result);return result;
}

export function virtualCursor(doc: CursorDocument, offset: number): number {
  for(const segment of doc.segments) {
    if(selectionSpans(segment.source).some(span=>contains(span,offset)))return segment.block===undefined?segment.start+offset-segment.source.start:segment.start;
  }
  const next=doc.segments.find(segment=>segment.source.start>offset);
  return next?.start??doc.segments.at(-1)?.start??0;
}
export function originalCursor(doc: CursorDocument, offset: number): number {
  const segment=doc.segments.find(segment=>contains(segment,offset));
  if(segment!==undefined)return segment.block===undefined?segment.source.start+offset-segment.start:segment.source.start;
  // Virtual line breaks are navigation separators, not source characters.
  return doc.segments.find(segment=>segment.start>offset)?.source.start??doc.segments.at(-1)?.source.start??0;
}
export function cursorLine(doc: CursorDocument, cursor: number): CursorLine | undefined {
  return doc.lines.find(line=>line.start<=cursor&&line.end>cursor);
}
export function cursorBlock(doc: CursorDocument, cursor: number): CursorBlock | undefined {
  return doc.segments.find(segment=>contains(segment,cursor))?.block;
}
export function cursorSelection(doc: CursorDocument, span: TextSpan): SourceSelection {
  const pieces=doc.segments.filter(segment=>overlaps(segment,span)).flatMap(segment=>segment.block===undefined
    ? [{start:segment.source.start+Math.max(0,span.start-segment.start),end:segment.source.end-Math.max(0,segment.end-span.end)}]
    : selectionSpans(segment.source));
  return compact(doc.source,pieces);
}
export function cursorGrapheme(doc: CursorDocument, cursor: number): SourceSelection {
  return cursorSelection(doc,sourceGrapheme(doc.index.text,cursor));
}
export function cursorCopyText(doc: CursorDocument, selection: SourceSelection, sourceUrl: string): string {
  const selected=doc.blocks.filter(block=>selectionSpans(selection).some(span=>selectionSpans(block).some(piece=>overlaps(span,piece))));
  if(selected.length===0)return selectionText(doc.source,selection);
  const emitted=new Set<CursorBlock>();const chunks:string[]=[];
  for(const span of selectionSpans(selection)) {
    const cuts=selected.flatMap(block=>selectionSpans(block).filter(piece=>overlaps(piece,span)).map(piece=>({...piece,block}))).sort((a,b)=>a.start-b.start);
    let start=span.start;
    for(const cut of cuts) {
      if(cut.start>start)chunks.push(doc.source.text.slice(start,cut.start));
      if(!emitted.has(cut.block)) {
        const link=new URL(sourceUrl);link.hash=`page=${cut.block.page}`;
        chunks.push(`\n\n${cut.block.caption}\n\n[${cut.block.label} · p. ${cut.block.page}](${link.href})\n\n`);emitted.add(cut.block);
      }
      start=Math.max(start,cut.end);
    }
    if(start<span.end)chunks.push(doc.source.text.slice(start,span.end));
  }
  return chunks.join('').trim();
}

/** Half-screen travel follows physical lines and counts a figure/table as one stop. */
export function moveCursorScreen(doc: CursorDocument, cursor: number, direction: number, points: number): number {
  const line=cursorLine(doc,cursor);if(line===undefined)return cursor;
  let at=line.number-1,travel=0;
  const heights=new Map(doc.source.pages.map(page=>[page.number,page.height]));
  while(at+(direction>0?1:-1)>=0&&at+(direction>0?1:-1)<doc.lines.length) {
    const before=doc.lines[at],next=doc.lines[at+(direction>0?1:-1)];
    if(before===undefined||next===undefined)break;
    const a=(before.rect.y_min+before.rect.y_max)/2,b=(next.rect.y_min+next.rect.y_max)/2;
    const delta=before.page===next.page?(direction*(b-a)>0?Math.abs(b-a):before.rect.y_max-before.rect.y_min)
      :direction>0?(heights.get(before.page)??792)-a+b:a+(heights.get(next.page)??792)-b;
    if(travel>0&&travel+delta>points)break;
    travel+=delta;at+=direction>0?1:-1;
    if(travel>=points)break;
  }
  const target=doc.lines[at];if(target===undefined)return cursor;
  const column=cursor-line.start;
  return sourceCursor(doc.index.text,Math.min(target.end-1,target.start+column));
}


/** Search/Visual mode can inspect an object's original text without changing its
 * one-stop navigation or the document's line numbering. */
export function cursorTextDocument(doc: CursorDocument, blockId: string | null): CursorDocument {
  if (blockId === null) return doc;
  return cursorDocument({...doc.source, figures: doc.source.figures.filter(figure => figure.id !== blockId)});
}
