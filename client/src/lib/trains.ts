import type { PendingMerge } from "./api";

// One product's outstanding merges, in the order the server will claim them.
// The first item is the head: the merge that is or will be worked next.
export interface MergeTrain {
  productId: string;
  items: PendingMerge[];
}

// The merge train is per product — one product's jam never holds another's —
// so the flat `pending_merges` list is split before it is drawn. The list
// arrives in `merge_sequence` order, which is the distribution order, and
// grouping preserves it both inside a train and between trains.
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
  return trains;
}
