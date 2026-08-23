import { cleanup, render } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import type { TaskSummary } from "./api";
import StatusTaskList from "./StatusTaskList.svelte";

const PRODUCT = "sunny-side/task-server";

function summary(
  id: string,
  status: string,
  kind = "normal",
  title = `task ${id}`,
): TaskSummary {
  return {
    id,
    title,
    status,
    kind,
    product_id: PRODUCT,
    priority: 0,
    updated_at: "2026-08-15T12:00:00Z",
  };
}

// Deliberately unordered, and carrying what the list must never show: a
// released task and an instant:merge task (DESIGN.md, Task list).
const ITEMS: TaskSummary[] = [
  summary("t-merged", "merged"),
  summary("t-draft", "draft"),
  summary("t-blocked", "blocked"),
  summary("t-approved", "approved"),
  summary("t-released", "released"),
  summary("t-wip", "wip"),
  summary("t-ready", "ready"),
  summary("m-1", "ready", "instant:merge"),
  summary("r-1", "ready", "review", "レビュー: t-done"),
  summary("t-done", "done"),
  summary("t-cancelled", "cancelled"),
  summary("t-dropped", "dropped"),
];

function groups(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>("[data-status]")];
}

function cardsOf(group: HTMLElement): HTMLElement[] {
  return [...group.querySelectorAll<HTMLElement>('a[href^="/tasks/"]')];
}

function focusableIn(root: HTMLElement): Element[] {
  return [
    ...root.querySelectorAll(
      'a[href], button, input, select, textarea, [tabindex]:not([tabindex="-1"])',
    ),
  ];
}

describe("StatusTaskList", () => {
  afterEach(cleanup);

  it("orders the groups with approved between done and merged, and drops the empty ones", () => {
    render(StatusTaskList, { props: { fetchState: "ready", items: ITEMS } });

    expect(groups().map((group) => group.dataset.status)).toEqual([
      "draft",
      "ready",
      "wip",
      "done",
      "approved",
      "merged",
      "blocked",
      "cancelled",
      "dropped",
    ]);
  });

  it("renders no group for a status nothing is in, and never one for released", () => {
    render(StatusTaskList, {
      props: {
        fetchState: "ready",
        items: [summary("t-approved", "approved"), summary("t-x", "released")],
      },
    });

    expect(groups().map((group) => group.dataset.status)).toEqual(["approved"]);
    expect(document.querySelector('[data-status="released"]')).toBeNull();
    expect(document.querySelector('a[href="/tasks/t-x"]')).toBeNull();
  });

  it("counts every card in the group's pill", () => {
    render(StatusTaskList, { props: { fetchState: "ready", items: ITEMS } });

    for (const group of groups()) {
      const pill = group.querySelector<HTMLElement>("[data-count]");
      expect(pill?.textContent?.trim()).toBe(String(cardsOf(group).length));
    }
    // ready holds the plain task and the review, but not the merge.
    const ready = groups().find((group) => group.dataset.status === "ready")!;
    expect(cardsOf(ready)).toHaveLength(2);
  });

  it("keeps a review task in its status group and marks it with a kind badge", () => {
    render(StatusTaskList, { props: { fetchState: "ready", items: ITEMS } });

    const ready = groups().find((group) => group.dataset.status === "ready")!;
    const review = ready.querySelector<HTMLElement>('a[href="/tasks/r-1"]');
    expect(review).not.toBeNull();
    expect(
      [...review!.querySelectorAll(".badge")].map((badge) =>
        badge.textContent?.trim(),
      ),
    ).toEqual(["review"]);

    const plain = ready.querySelector<HTMLElement>('a[href="/tasks/t-ready"]');
    expect(plain).not.toBeNull();
    expect(plain!.querySelectorAll(".badge")).toHaveLength(0);
  });

  it("hides instant:merge tasks, which the control panel already draws", () => {
    render(StatusTaskList, { props: { fetchState: "ready", items: ITEMS } });

    expect(document.querySelector('a[href="/tasks/m-1"]')).toBeNull();
    for (const group of groups()) {
      for (const card of cardsOf(group)) {
        expect(card.textContent).not.toContain("instant:merge");
      }
    }
  });

  it("hides the tasks the panel draws in its readouts, and nothing more", () => {
    // The pending review sits in the review queue; the done task with no
    // review and the approved task with no merge sit in reconciliation.
    render(StatusTaskList, {
      props: {
        fetchState: "ready",
        items: ITEMS,
        drawnElsewhere: ["r-1", "t-done", "t-approved"],
      },
    });

    for (const id of ["r-1", "t-done", "t-approved"]) {
      expect(document.querySelector(`a[href="/tasks/${id}"]`)).toBeNull();
    }
    expect(groups().map((group) => group.dataset.status)).toEqual([
      "draft",
      "ready",
      "wip",
      "merged",
      "blocked",
      "cancelled",
      "dropped",
    ]);
    const ready = groups().find((group) => group.dataset.status === "ready")!;
    expect(cardsOf(ready).map((card) => card.getAttribute("href"))).toEqual([
      "/tasks/t-ready",
    ]);
    expect(
      ready.querySelector<HTMLElement>("[data-count]")?.textContent?.trim(),
    ).toBe("1");
  });

  it("keeps a review the panel no longer draws in its status group", () => {
    // Same rule, other way round: once the review is not pending, the review
    // queue is not drawing it, so it falls back into its group with its badge.
    render(StatusTaskList, {
      props: {
        fetchState: "ready",
        items: [summary("r-1", "done", "review", "レビュー: t-done")],
        drawnElsewhere: [],
      },
    });

    const done = groups().find((group) => group.dataset.status === "done")!;
    const card = done.querySelector<HTMLElement>('a[href="/tasks/r-1"]')!;
    expect(card).not.toBeNull();
    expect(card.querySelector(".badge")?.textContent?.trim()).toBe("review");
  });

  it("gives a card exactly one focusable element, the link itself", () => {
    render(StatusTaskList, { props: { fetchState: "ready", items: ITEMS } });

    const ready = groups().find((group) => group.dataset.status === "ready")!;
    for (const card of cardsOf(ready)) {
      expect(focusableIn(card)).toHaveLength(0);
      expect(card.hasAttribute("tabindex")).toBe(false);
    }
    expect(focusableIn(ready)).toHaveLength(cardsOf(ready).length);
  });
});
