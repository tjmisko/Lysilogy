import type { ListViewState } from "../../lib/listViewModel";
import type { ListActions } from "../../lib/useListDetail";
import type { ListDetail } from "../../types";

/**
 * What the list page hands every view. The page owns the cursor (`selectedId`, a node id, or
 * `group:<key>` for a topic header) and the keys that work on any node: j/k, 1-7, v, r, e, x,
 * u, a, A, O, T, E, X, C and the Work panel. A view draws the projection and may bind keys of its
 * own while `keyboardEnabled`; the Review view (7) owns `a`, `x` and `c`, and the Acquire view
 * (6) owns `f` and `F`, which the page then leaves alone.
 */
export type ListViewProps = {
  detail: ListDetail;
  selectedId: string | null;
  onSelect: (cursorId: string) => void;
  onOpen: (cursorId: string) => void;
  actions: ListActions;
  keyboardEnabled: boolean;
  viewState: ListViewState;
  /** Titles of Works outside the list that rows mention, by Work id. */
  workTitles: Readonly<Record<string, string>>;
  /** Topic tag colors by lowercase tag name; a topic whose label matches a tag wears its color. */
  tagColors: Readonly<Record<string, string>>;
  /** The node whose note is being edited in place (`e`); only Sequence draws the input. */
  editingNoteId: string | null;
  onSaveNote: (nodeId: string, note: string) => void;
  onCancelNote: () => void;
};
