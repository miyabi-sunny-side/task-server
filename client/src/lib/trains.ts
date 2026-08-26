import type { PendingMerge } from "./api";

// One product's outstanding merges. The order says state, not sequence: the
// merge holding the product up comes first, and the rest follow as a set
// (DESIGN.md, Merge trains). Nothing here promises which merge runs next.
export interface MergeTrain {
  productId: string;
  items: PendingMerge[];
}

// The merge train is per product — one product's jam never holds another's —
// so the flat `pending_merges` list is split before it is drawn. Within a
// train the holder is lifted to the front; `sort` is stable, so everything
// else keeps the server's own stable order and the list does not shuffle
// under the reader between two loads.
export function mergeTrains(pending: PendingMerge[]): MergeTrain[] {
  const trains: MergeTrain[] = [];
  const byProduct = new Map<string, MergeTrain>();
  for (const item of pending) {
    let train = byProduct.get(item.product_id);
    if (!train) {
      train = { productId: item.product_id, items: [] };
      byProduct.set(item.product_id, train);
      trains.push(train);
    }
    train.items.push(item);
  }
  for (const train of trains) {
    train.items.sort((a, b) => holds(b) - holds(a));
  }
  return trains;
}

// Whether this merge is the one holding its product: running, or stopped and
// waiting for a person. Only these two states keep the rest of the product
// back, so only these two earn the front of the list.
function holds(merge: PendingMerge): number {
  return merge.status === "wip" || merge.status === "blocked" ? 1 : 0;
}
