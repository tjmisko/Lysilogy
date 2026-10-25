import type { ComponentType } from "react";

import { viewOrder, type ListViewState } from "../../lib/listViewModel";
import { LIST_VIEWS, type ListViewId } from "../../lib/listViews";
import type { ListDetail } from "../../types";
import { Acquire } from "./Acquire";
import { GraphDeferred } from "./GraphDeferred";
import { NextUp } from "./NextUp";
import type { ListViewProps } from "./props";
import { Review } from "./Review";
import { Sequence } from "./Sequence";
import { Timeline } from "./Timeline";
import { Topics } from "./Topics";

export type { ListViewProps } from "./props";

export type ListViewDef = {
  id: ListViewId;
  name: string;
  component: ComponentType<ListViewProps>;
  /** The ids the cursor visits, in display order. */
  order: (detail: ListDetail, state: ListViewState) => string[];
};

const COMPONENTS: Readonly<Record<ListViewId, ComponentType<ListViewProps>>> = {
  1: Sequence,
  2: Topics,
  3: Timeline,
  4: GraphDeferred,
  5: NextUp,
  6: Acquire,
  7: Review,
};

/** The seven list views by number. Replace a component here to swap a view. */
export const LIST_VIEW_DEFS: readonly ListViewDef[] = LIST_VIEWS.map(({ id, name }) => ({
  id,
  name,
  component: COMPONENTS[id],
  order: (detail, state) => viewOrder(id, detail, state),
}));

export function viewDef(id: ListViewId): ListViewDef {
  return LIST_VIEW_DEFS.find((def) => def.id === id) ?? LIST_VIEW_DEFS[0] as ListViewDef;
}
