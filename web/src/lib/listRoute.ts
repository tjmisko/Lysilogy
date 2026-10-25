// Routes and colon commands of the reading-list pages (spec: UI, Keymap). Pure, so Node tests
// can drive them.

import { parseView, type ListViewId } from "./listViews.ts";

export type AppRoute =
  | { kind: "lists" }
  | { kind: "list"; listId: string; view: ListViewId }
  | { kind: "history" };

/** The list-area route a hash names; null for home, a paper, or anything unknown. */
export function parseAppRoute(hash: string): AppRoute | null {
  const bare = hash.replace(/^#/u, "");
  const params = new URLSearchParams(bare);
  if (params.has("paper")) return null;
  const listId = params.get("list");
  if (listId !== null) {
    if (listId.trim() === "") return null;
    return { kind: "list", listId, view: parseView(params.get("view")) };
  }
  const first = bare.split("&")[0];
  if (first === "lists") return { kind: "lists" };
  if (first === "history") return { kind: "history" };
  return null;
}

export function routeHash(route: AppRoute): string {
  switch (route.kind) {
    case "lists": return "#lists";
    case "history": return "#history";
    case "list": return `#list=${encodeURIComponent(route.listId)}&view=${route.view}`;
  }
}

export function sameRoute(left: AppRoute | null, right: AppRoute | null): boolean {
  if (left === null || right === null) return left === right;
  return routeHash(left) === routeHash(right);
}

export type ReadingCommandStatus = "unread" | "reading" | "read";
export type OrderCommandStrategy = "topological" | "year" | "foundational_first";

export type AppCommand =
  | { kind: "lists" }
  | { kind: "history" }
  | { kind: "list"; title: string }
  | { kind: "list-new"; title: string }
  | { kind: "generate"; goal: string }
  | { kind: "delete-list"; title: string }
  | { kind: "read-status"; status: ReadingCommandStatus }
  | { kind: "order"; strategy: OrderCommandStrategy }
  | { kind: "verify" }
  | { kind: "error"; message: string };

/** Length in characters (code points), not UTF-16 units. */
const characters = (text: string): number => Array.from(text).length;

const GOAL_LIMIT = 2000;
const TITLE_LIMIT = 120;
const ORDER_STRATEGIES: Readonly<Record<string, OrderCommandStrategy>> = {
  topological: "topological",
  year: "year",
  foundational_first: "foundational_first",
  "foundational-first": "foundational_first",
};

/**
 * A reading-list colon command, or null when `raw` names some other command. The argument keeps
 * its case: `:list RLHF basics` looks a title up, so it must not be lowercased.
 */
export function parseAppCommand(raw: string): AppCommand | null {
  const text = raw.trim();
  const split = /^(\S+)(?:\s+(.*))?$/su.exec(text);
  if (split === null) return null;
  const name = (split[1] ?? "").toLowerCase();
  const argument = (split[2] ?? "").trim();
  const bare = (command: AppCommand): AppCommand =>
    argument === "" ? command : { kind: "error", message: `:${name} takes no argument.` };
  switch (name) {
    case "lists": return bare({ kind: "lists" });
    case "history": return bare({ kind: "history" });
    case "verify": return bare({ kind: "verify" });
    case "read":
    case "reading":
    case "unread": return bare({ kind: "read-status", status: name });
    case "list":
    case "list-new":
    case "delete-list": {
      if (argument === "") return { kind: "error", message: `Give a list title: :${name} <title>.` };
      if (name !== "list" && characters(argument) > TITLE_LIMIT) {
        return { kind: "error", message: `A list title is at most ${TITLE_LIMIT} characters.` };
      }
      return { kind: name, title: argument };
    }
    case "generate": {
      if (argument === "") return { kind: "error", message: "Give a learning goal: :generate <goal>." };
      if (characters(argument) > GOAL_LIMIT) {
        return { kind: "error", message: `A goal is at most ${GOAL_LIMIT} characters.` };
      }
      return { kind: "generate", goal: argument };
    }
    case "order": {
      const strategy = Object.hasOwn(ORDER_STRATEGIES, argument.toLowerCase()) ? ORDER_STRATEGIES[argument.toLowerCase()] : undefined;
      if (strategy === undefined) return { kind: "error", message: "Use :order topological, year, or foundational-first." };
      return { kind: "order", strategy };
    }
    default: return null;
  }
}

/** The command names that take an argument; CommandMenu runs `name rest` as typed instead of matching. */
export const ARGUMENT_COMMANDS: readonly string[] = ["list", "list-new", "generate", "delete-list", "order"];

/**
 * Resolve a title the way the CLI does: an exact id, else a unique case-insensitive title
 * prefix. An exact (case-insensitive) title wins over longer prefix matches.
 */
export function resolveListTitle<T extends { id: string; title: string }>(
  lists: readonly T[],
  text: string,
): { kind: "found"; list: T } | { kind: "missing" } | { kind: "ambiguous"; matches: T[] } {
  const wanted = text.trim().toLowerCase();
  if (wanted === "") return { kind: "missing" };
  const byId = lists.find((list) => list.id === text.trim());
  if (byId !== undefined) return { kind: "found", list: byId };
  const exact = lists.filter((list) => list.title.toLowerCase() === wanted);
  if (exact.length === 1 && exact[0] !== undefined) return { kind: "found", list: exact[0] };
  const prefixed = lists.filter((list) => list.title.toLowerCase().startsWith(wanted));
  if (prefixed.length === 1 && prefixed[0] !== undefined) return { kind: "found", list: prefixed[0] };
  return prefixed.length === 0 ? { kind: "missing" } : { kind: "ambiguous", matches: prefixed };
}
