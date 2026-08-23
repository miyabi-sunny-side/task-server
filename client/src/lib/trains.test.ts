import { describe, expect, it } from "vitest";

import type { PendingMerge } from "./api";
import { mergeTrains } from "./trains";

function merge(id: string, productId: string, status = "ready"): PendingMerge {
  return {
    id,
    title: `merge ${id}`,
    status,
    kind: "instant:merge",
    product_id: productId,
    priority: 0,
    updated_at: "2026-08-15T12:00:00Z",
    verification: null,
  };
}

// The server hands `pending_merges` over in merge_sequence order, so this
// array is the distribution order read straight through.
const PENDING: PendingMerge[] = [
  merge("m-1", "sunny-side/one"),
  merge("m-2", "sunny-side/two"),
  merge("m-3", "sunny-side/one"),
  merge("m-4", "sunny-side/two"),
  merge("m-5", "sunny-side/one"),
];

describe("mergeTrains", () => {
  it("splits one flat list into a train per product", () => {
    expect(
      mergeTrains(PENDING).map((train) => [
        train.productId,
        train.items.map((item) => item.id),
      ]),
    ).toEqual([
      ["sunny-side/one", ["m-1", "m-3", "m-5"]],
      ["sunny-side/two", ["m-2", "m-4"]],
    ]);
  });

  it("orders the trains by the head each product first put on the wire", () => {
    const reordered = [PENDING[1], PENDING[0], PENDING[2]];

    expect(mergeTrains(reordered).map((train) => train.productId)).toEqual([
      "sunny-side/two",
      "sunny-side/one",
    ]);
  });

  it("returns nothing for an empty queue", () => {
    expect(mergeTrains([])).toEqual([]);
  });
});
