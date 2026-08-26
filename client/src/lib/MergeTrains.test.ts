import { cleanup, render } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import type { PendingMerge } from "./api";
import MergeTrains from "./MergeTrains.svelte";

const ONE = "sunny-side/one";
const TWO = "sunny-side/two";

// The stop reason travels on the merge itself, exactly as /api/control sends
// it: a running merge carries `null`, a blocked one carries what the worker
// wrote when it could not integrate the branch.
function merge(
  id: string,
  productId: string,
  status = "ready",
  verification: string | null = null,
): PendingMerge {
  return {
    id,
    title: `merge ${id}`,
    status,
    kind: "instant:merge",
    product_id: productId,
    priority: 0,
    updated_at: "2026-08-15T12:00:00Z",
    verification,
  };
}

// Interleaved on the wire, exactly as the server hands them over.
const PENDING: PendingMerge[] = [
  merge("m-1", ONE),
  merge("m-2", TWO),
  merge("m-3", ONE),
  merge("m-4", TWO),
  merge("m-5", ONE),
];

function trains(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>("[data-train]")];
}

function trainOf(productId: string): HTMLElement {
  const found = document.querySelector<HTMLElement>(
    `[data-train="${productId}"]`,
  );
  if (!found) {
    throw new Error(`no train for ${productId}`);
  }
  return found;
}

function cardsOf(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>('a[href^="/tasks/"]')];
}

describe("MergeTrains", () => {
  afterEach(cleanup);

  it("draws one group per product, captioned by product id with its count", () => {
    render(MergeTrains, { props: { pending: PENDING } });

    expect(trains().map((train) => train.dataset.train)).toEqual([ONE, TWO]);
    for (const [productId, count] of [
      [ONE, 3],
      [TWO, 2],
    ] as const) {
      const train = trainOf(productId);
      expect(train.textContent).toContain(productId);
      expect(
        train.querySelector<HTMLElement>("[data-count]")?.textContent?.trim(),
      ).toBe(String(count));
      expect(cardsOf(train)).toHaveLength(count);
    }
  });

  // Nothing here is holding a product, so no card earns the front and the
  // server's own stable order survives — the list must not shuffle under the
  // reader between two loads.
  it("keeps the server's order when no merge is holding a product", () => {
    render(MergeTrains, { props: { pending: PENDING } });

    expect(
      cardsOf(trainOf(ONE)).map((card) => card.getAttribute("href")),
    ).toEqual(["/tasks/m-1", "/tasks/m-3", "/tasks/m-5"]);
    expect(
      cardsOf(trainOf(TWO)).map((card) => card.getAttribute("href")),
    ).toEqual(["/tasks/m-2", "/tasks/m-4"]);
  });

  it("renders nothing at all when no merge is outstanding", () => {
    render(MergeTrains, { props: { pending: [] } });

    expect(document.querySelector("[data-block='trains']")).toBeNull();
    expect(trains()).toHaveLength(0);
  });

  it("names the stopped merge's cause and how many wait on it", () => {
    const jammed = [
      merge("m-1", ONE, "blocked", "rebase conflict:\n  src/task.rs"),
      merge("m-3", ONE),
      merge("m-5", ONE),
      merge("m-2", TWO),
      merge("m-4", TWO),
    ];

    render(MergeTrains, { props: { pending: jammed } });

    const jam = trainOf(ONE);
    const reason = jam.querySelector<HTMLElement>("[data-reason]");
    expect(reason?.textContent).toContain("rebase conflict:");
    expect(reason?.textContent).toContain("src/task.rs");
    // The reason belongs to the stopped merge itself, not to a position, and
    // no other card carries one.
    const cards = cardsOf(jam);
    expect(cards[0].contains(reason)).toBe(true);
    expect(jam.querySelectorAll("[data-reason]")).toHaveLength(1);
    expect(jam.textContent).toContain("他 2 件が待機中");

    // The other product's train is untouched by the jam.
    const running = trainOf(TWO);
    expect(running.querySelector("[data-reason]")).toBeNull();
    expect(running.textContent).not.toContain("待機中");
  });

  it("puts the merge that holds the product before the ones that wait", () => {
    render(MergeTrains, {
      props: {
        pending: [
          merge("m-1", ONE),
          merge("m-3", ONE, "blocked", "check failed"),
          merge("m-5", ONE),
        ],
      },
    });

    const cards = cardsOf(trainOf(ONE));
    expect(cards[0].textContent).toContain("m-3");
    for (const behind of cards.slice(1)) {
      expect(
        cards[0].compareDocumentPosition(behind) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
    }
  });

  it("omits the waiting caption when nothing waits on the stopped merge", () => {
    render(MergeTrains, {
      props: { pending: [merge("m-1", ONE, "blocked", "check failed")] },
    });

    expect(trainOf(ONE).querySelector("[data-reason]")).not.toBeNull();
    expect(trainOf(ONE).textContent).not.toContain("待機中");
  });

  it("shows no reason while the holder is running, even if one is on the row", () => {
    render(MergeTrains, {
      props: {
        pending: [
          merge("m-1", ONE, "ready", "stale reason"),
          merge("m-3", ONE),
        ],
      },
    });

    expect(document.querySelector("[data-reason]")).toBeNull();
    expect(document.body.textContent).not.toContain("stale reason");
  });

  it("wears the status badge and adds no second focus stop inside a card", () => {
    render(MergeTrains, {
      props: {
        pending: [
          merge("m-1", ONE, "blocked", "rebase conflict"),
          merge("m-3", ONE),
        ],
      },
    });

    const cards = cardsOf(trainOf(ONE));
    expect(
      cards.map((card) => card.querySelector(".badge")?.textContent?.trim()),
    ).toEqual(["blocked", "ready"]);
    for (const card of cards) {
      expect(
        card.querySelectorAll(
          'a[href], button, input, select, textarea, [tabindex]:not([tabindex="-1"])',
        ),
      ).toHaveLength(0);
    }
  });

  it("writes no caption that merely repeats a status", () => {
    render(MergeTrains, {
      props: {
        pending: [
          merge("m-1", ONE, "blocked", "rebase conflict"),
          merge("m-3", ONE),
        ],
      },
    });

    const captions = [...trainOf(ONE).querySelectorAll<HTMLElement>("p")].map(
      (caption) => caption.textContent?.trim() ?? "",
    );
    for (const caption of captions) {
      expect(["ready", "blocked", "merge 進行中"]).not.toContain(caption);
    }
  });
});
