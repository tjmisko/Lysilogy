// The declared keymap of the reading-list surfaces and the keys they must not collide with
// (spec: Keymap). HelpOverlay renders from it and web/scripts/keymap.test.mjs audits it, so a new
// binding is one row here and the audit says where it clashes.

/** A key as `KeyboardEvent.key` names it, with `Ctrl-` for the control modifier. Sequences are space separated ("g l"). */
export type KeyChord = string;

export type KeyBinding = {
  /** Alternatives that do the same thing. */
  keys: readonly KeyChord[];
  action: string;
  /** Sub-states of the context in which the binding is live (a list view, the graph's Work panel); every state when absent. */
  scopes?: readonly string[];
  /** The one app-wide binding of this key, repeated by a page that spells it out; it never collides with the global table. */
  global?: true;
};

export type KeyContext = {
  id: string;
  title: string;
  /** Shown in the key map; contexts that only exist for the audit stay out of it. */
  help: boolean;
  bindings: readonly KeyBinding[];
};

const ALL_VIEWS = ["1", "2", "3", "4", "5", "6", "7"] as const;
const without = (...skip: string[]): string[] => ALL_VIEWS.filter((view) => !skip.includes(view));

const GLOBAL: KeyContext = {
  id: "global",
  title: "Everywhere",
  help: false,
  bindings: [
    { keys: ["g l"], action: "Lists index" },
    { keys: ["g r"], action: "Reading history" },
    { keys: ["Y"], action: "Add the current paper or Work to a list" },
    { keys: ["M"], action: "Lists this paper or Work is in" },
  ],
};

const LISTS: KeyContext = {
  id: "lists",
  title: "Lists",
  help: true,
  bindings: [
    { keys: ["j", "ArrowDown"], action: "Next list" },
    { keys: ["k", "ArrowUp"], action: "Previous list" },
    { keys: ["g g"], action: "First list" },
    { keys: ["G"], action: "Last list" },
    { keys: ["Ctrl-d", "Ctrl-u"], action: "Half-screen down / up" },
    { keys: ["Enter", "o"], action: "Open the list" },
    { keys: ["/"], action: "Filter by title" },
    { keys: ["n"], action: "New list (inline title)" },
    { keys: ["R"], action: "Rename the selected list" },
    { keys: ["A"], action: "Archive / unarchive the selected list" },
    { keys: ["z a"], action: "Show / hide archived lists" },
    { keys: ["q", "Escape"], action: "Clear the filter, then back" },
  ],
};

const LIST_PAGE: KeyContext = {
  id: "list-page",
  title: "List page",
  help: true,
  bindings: [
    { keys: ["j", "ArrowDown"], action: "Next row" },
    { keys: ["k", "ArrowUp"], action: "Previous row" },
    { keys: ["g g"], action: "First row" },
    { keys: ["G"], action: "Last row" },
    { keys: ["Ctrl-d", "Ctrl-u"], action: "Half-screen down / up" },
    { keys: ["1", "2", "3", "4", "5", "6", "7"], action: "Switch view: Sequence, Topics, Timeline, Graph (not in this build), Next up, Acquire, Review" },
    { keys: ["v"], action: "Cycle views" },
    { keys: ["Enter", "o"], action: "Open: reader if held, else the Work panel; with E pending, draw the edge" },
    { keys: ["a"], action: "Add by name (library, graph, or a claim to verify)", scopes: without("7") },
    { keys: ["A"], action: "Add from the selected Work's references" },
    { keys: ["O"], action: "New topic" },
    { keys: ["T"], action: "File the selected Work under topics" },
    { keys: ["E"], action: "Draw a prerequisite edge: E, move to the target, Enter; Esc cancels" },
    { keys: ["X"], action: "Remove an edge of the selected node" },
    { keys: ["C"], action: "Context links to papers you have read" },
    { keys: ["d d"], action: "Cut a row, then p / P to put below / above", scopes: ["1"] },
    { keys: ["p"], action: "Put the cut row below" },
    { keys: ["P"], action: "Put the cut row above" },
    { keys: ["J"], action: "Move the row down" },
    { keys: ["K"], action: "Move the row up" },
    { keys: ["x"], action: "Remove the node (u undoes)", scopes: without("7") },
    { keys: ["u"], action: "Undo the last removal" },
    { keys: ["r"], action: "Cycle status: unread, reading, read" },
    { keys: ["e"], action: "Edit the note (Enter saves, Esc cancels)" },
    { keys: ["s"], action: "Cycle sort (view only)", scopes: ["1"] },
    { keys: ["S"], action: "Reverse the sort", scopes: ["1"] },
    { keys: ["/"], action: "Filter rows", scopes: ["1"] },
    { keys: ["z g"], action: "Group by topic", scopes: ["1"] },
    { keys: ["z o", "z c", "z a", "z R", "z M"], action: "Fold topics: open, close, toggle, open all, close all", scopes: ["1", "2"] },
    { keys: ["h", "ArrowLeft"], action: "Previous column (Topics, Timeline)", scopes: ["2", "3"] },
    { keys: ["l", "ArrowRight"], action: "Next column (Topics, Timeline)", scopes: ["2", "3"] },
    { keys: ["f", "F"], action: "Fetch the Work open in the panel", scopes: without("6") },
    { keys: ["f"], action: "Acquire: fetch the selected Work", scopes: ["6"] },
    { keys: ["F"], action: "Acquire: fetch every missing Work", scopes: ["6"] },
    { keys: ["a"], action: "Review: accept the proposed item or link", scopes: ["7"] },
    { keys: ["x"], action: "Review: reject the proposed item or link", scopes: ["7"] },
    { keys: ["c"], action: "Review: resolve a claim to a candidate; c again for the next", scopes: ["7"] },
    { keys: ["q", "Escape"], action: "Leave a mode, then back" },
  ],
};

// The candidate chooser is modal: while it is open the Review view takes these before the page does.
const LIST_REVIEW_CHOOSER: KeyContext = {
  id: "list-review-chooser",
  title: "List page · Review chooser",
  help: false,
  bindings: [
    { keys: ["Enter"], action: "Confirm the open candidate chooser" },
    { keys: ["Escape"], action: "Close the candidate chooser" },
    { keys: ["c"], action: "Next candidate" },
  ],
};

// The two dialogs own the keyboard while open (useDialogKeys), so each is a context of its own.
const PICKER: KeyContext = {
  id: "picker",
  title: "Add to a list (Y)",
  help: true,
  bindings: [
    { keys: ["ArrowDown", "Ctrl-n"], action: "Next list" },
    { keys: ["ArrowUp", "Ctrl-p"], action: "Previous list" },
    { keys: ["Enter"], action: "Add to the highlighted list" },
    { keys: ["Ctrl-Enter"], action: "Create a list from the typed title and add" },
    { keys: ["Tab"], action: "Choose a topic of the highlighted list first" },
    { keys: ["j", "k"], action: "Move in the topic choice (they type into the search on the list stage)" },
    { keys: ["Escape"], action: "Back from the topic choice, then cancel" },
  ],
};

const MEMBERSHIP: KeyContext = {
  id: "membership",
  title: "Lists this paper is on (M)",
  help: true,
  bindings: [
    { keys: ["j", "ArrowDown"], action: "Next list" },
    { keys: ["k", "ArrowUp"], action: "Previous list" },
    { keys: ["Enter"], action: "Open the list" },
    { keys: ["x"], action: "Remove from the list" },
    { keys: ["u"], action: "Undo the removal" },
    { keys: ["M", "q", "Escape"], action: "Close", global: true },
  ],
};

const HISTORY: KeyContext = {
  id: "history",
  title: "History",
  help: true,
  bindings: [
    { keys: ["j", "ArrowDown"], action: "Next row (more days load at the end)" },
    { keys: ["k", "ArrowUp"], action: "Previous row" },
    { keys: ["g g"], action: "First row" },
    { keys: ["G"], action: "Last row" },
    { keys: ["Ctrl-d", "Ctrl-u"], action: "Half-screen down / up" },
    { keys: ["[", "]"], action: "Previous / next day" },
    { keys: ["Enter", "o"], action: "Reopen at the saved position" },
    { keys: ["Y"], action: "Add to a list", global: true },
    { keys: ["/"], action: "Filter by text" },
    { keys: ["1", "2", "3", "4", "5"], action: "Filter kinds: papers, sections, searches, graph, lists" },
    { keys: ["z o", "z c", "z a", "z R", "z M"], action: "Expand / collapse a session; all open / all closed" },
    { keys: ["I"], action: "Inspected, never opened" },
    { keys: ["Escape"], action: "Leave inspected, clear the filter, clear kinds" },
  ],
};

// The graph keys the lists work touches or sits beside. Scopes: radial, year, work (Work panel open on radial).
const GRAPH: KeyContext = {
  id: "graph",
  title: "Literature graph",
  help: false,
  bindings: [
    { keys: ["j", "k"], action: "Move across cited works", scopes: ["radial", "work", "year"] },
    { keys: ["h", "l"], action: "Move across cited works", scopes: ["radial", "year"] },
    { keys: ["Enter"], action: "Open a work, or expand a cluster", scopes: ["radial", "year"] },
    { keys: ["c"], action: "Collapse every expanded cluster", scopes: ["radial"] },
    { keys: ["+", "-"], action: "Show more / fewer ranked works", scopes: ["radial"] },
    { keys: ["p", "d", "h"], action: "Pin / downplay / hide", scopes: ["work"] },
    { keys: ["t"], action: "Tag input", scopes: ["work"] },
    { keys: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"], action: "Tag filter", scopes: ["radial"] },
    { keys: ["a", "x", "e"], action: "Accept / reject / edit the citing reason", scopes: ["work"] },
    { keys: ["g"], action: "Show the citing sentence", scopes: ["work"] },
    { keys: ["f"], action: "Fetch an open-access copy", scopes: ["work"] },
    { keys: ["s"], action: "Sources", scopes: ["work"] },
    { keys: ["w"], action: "Expand the work's references in place", scopes: ["radial", "work"] },
    { keys: ["y"], action: "Add the selected Work to the current list", scopes: ["radial", "work"] },
    { keys: ["R"], action: "Show / hide the list sidebar", scopes: ["radial", "work"] },
    { keys: ["v"], action: "Radial graph / year histogram" },
  ],
};

// Reader and home keys that sit under the global table. Not exhaustive: the keys the global table could shadow.
// The glossary's bare g (opened after a pause) is left out: it is a prefix of every g sequence by design.
const READER: KeyContext = {
  id: "reader",
  title: "Reader and home",
  help: false,
  bindings: [
    { keys: ["h", "j", "k", "l", "ArrowLeft", "ArrowDown", "ArrowUp", "ArrowRight"], action: "Move" },
    { keys: ["g g"], action: "First tile" },
    { keys: ["G"], action: "Last tile" },
    { keys: ["g W", "g H"], action: "Fit visible PDF content" },
    { keys: ["[", "]"], action: "Previous / next paper or page" },
    { keys: ["/"], action: "Search" },
    { keys: ["b"], action: "Library" },
    { keys: ["d"], action: "Digest" },
    { keys: ["L"], action: "Literature graph" },
    { keys: ["Enter", "o"], action: "Open" },
    { keys: ["m", "p", "f", "2", "W", "H", "P", "R", "I", "C", "+", "-"], action: "Reader controls" },
    { keys: ["Q", "T", "E", "?", ":"], action: "Application panels" },
    { keys: ["q", "Escape"], action: "Back" },
  ],
};

export const KEYMAP: readonly KeyContext[] = [GLOBAL, LISTS, LIST_PAGE, LIST_REVIEW_CHOOSER, PICKER, MEMBERSHIP, HISTORY, GRAPH, READER];

/**
 * Collisions that are deliberate. `g l` / `g r` are caught in the capture phase and leave a bare
 * `g` untouched, so a surface that acts on `g` alone still does; the second key then belongs to the
 * list route.
 */
export const KEYMAP_WAIVERS: readonly { context: string; keys: KeyChord; against: KeyChord; reason: string }[] = [
  { context: "graph", keys: "g", against: "g l", reason: "g shows the citing sentence immediately; a second l or r is taken by the list route." },
  { context: "graph", keys: "g", against: "g r", reason: "As g l: the history route takes the r." },
];

/** Contexts the key map shows, as `[keys, action]` rows with alternatives joined by " / ". */
export function helpGroups(): { title: string; commands: [string, string][] }[] {
  return KEYMAP.filter((context) => context.help).map((context) => ({
    title: context.title,
    commands: context.bindings.map((binding): [string, string] => [binding.keys.join(" / "), binding.action]),
  }));
}

export type KeyCollision = { context: string; keys: KeyChord; against: KeyChord; other: string };

const tokens = (chord: KeyChord): string[] => chord.split(" ");

/** The same chord, or one that is a strict prefix of the other (so the shorter fires before the longer completes). */
function clash(left: KeyChord, right: KeyChord): boolean {
  const a = tokens(left);
  const b = tokens(right);
  const shared = Math.min(a.length, b.length);
  return a.slice(0, shared).every((token, index) => token === b[index]);
}

const scopesOverlap = (left: KeyBinding, right: KeyBinding): boolean =>
  left.scopes === undefined || right.scopes === undefined || left.scopes.some((scope) => right.scopes?.includes(scope));

/**
 * Every pair of bindings that cannot both fire: within one context when their scopes overlap, and
 * between the global table and every other context. Waived pairs are left out.
 */
export function findCollisions(
  contexts: readonly KeyContext[] = KEYMAP,
  waivers: typeof KEYMAP_WAIVERS = KEYMAP_WAIVERS,
): KeyCollision[] {
  const found: KeyCollision[] = [];
  const waived = (context: string, keys: KeyChord, against: KeyChord): boolean =>
    waivers.some((waiver) => waiver.context === context && waiver.keys === keys && waiver.against === against);
  const compare = (context: string, left: KeyBinding, other: string, right: KeyBinding): void => {
    for (const keys of left.keys) {
      for (const against of right.keys) {
        if (!clash(keys, against) || waived(context, keys, against)) continue;
        found.push({ context, keys, against, other });
      }
    }
  };
  for (const context of contexts) {
    context.bindings.forEach((left, at) => {
      context.bindings.slice(at + 1).forEach((right) => {
        if (scopesOverlap(left, right)) compare(context.id, left, context.id, right);
      });
    });
    if (context.id === "global") continue;
    const global = contexts.find((entry) => entry.id === "global");
    for (const left of context.bindings) {
      if (left.global === true) continue;
      for (const right of global?.bindings ?? []) compare(context.id, left, "global", right);
    }
  }
  return found;
}
