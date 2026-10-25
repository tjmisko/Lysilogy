// Window events that connect the app shell, the list pages and the pages that add to lists.

import type { AppCommand } from "./listRoute.ts";

/** Ask App to open the list picker (`Y`) for a paper or a Work. Detail: PickerTarget. */
export const OPEN_LIST_PICKER_EVENT = "lysilogy:open-list-picker";
/** Ask App to open the memberships popover (`M`). Detail: PickerTarget. */
export const OPEN_LIST_MEMBERSHIP_EVENT = "lysilogy:open-list-membership";
/** Sent after anything added to or removed from a list, so open pages reload. */
export const LISTS_CHANGED_EVENT = "lysilogy:lists-changed";
/** Sent by App to the open list page for a colon command that acts on it. Detail: AppCommand. */
export const LIST_COMMAND_EVENT = "lysilogy:list-command";

/** What `Y` and `M` act on: a held paper, a Work, or both. */
export type PickerTarget = {
  paperId?: string;
  workId?: string;
  /** For the dialog heading; the dialog falls back to a generic label. */
  title?: string;
};

export function requestListPicker(target: PickerTarget): void {
  window.dispatchEvent(new CustomEvent<PickerTarget>(OPEN_LIST_PICKER_EVENT, { detail: target }));
}

export function requestListMembership(target: PickerTarget): void {
  window.dispatchEvent(new CustomEvent<PickerTarget>(OPEN_LIST_MEMBERSHIP_EVENT, { detail: target }));
}

export function announceListsChanged(): void {
  window.dispatchEvent(new CustomEvent(LISTS_CHANGED_EVENT));
}

export function sendListCommand(command: AppCommand): void {
  window.dispatchEvent(new CustomEvent<AppCommand>(LIST_COMMAND_EVENT, { detail: command }));
}
