import { cleanup, render, screen } from "@testing-library/svelte";
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
// released task and two subtasks (review, instant:merge) (DESIGN.md, Task list).
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
  summary("w-1", "ready", "rework", "手直し: t-done"),
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

  it("orders only the active statuses and drops the empty groups", () => {
    render(StatusTaskList, { props: { fetchState: "ready", items: ITEMS } });

    expect(groups().map((group) => group.dataset.status)).toEqual([
      "draft",
      "ready",
      "wip",
      "blocked",
    ]);
    // Called-off work leaves this page for the closed one, like released.
    for (const status of ["cancelled", "dropped"]) {
      expect(document.querySelector(`[data-status="${status}"]`)).toBeNull();
    }
  });

  it("renders no group for a status nothing is in, and never one for released", () => {
    render(StatusTaskList, {
      props: {
        fetchState: "ready",
        items: [summary("t-approved", "approved"), summary("t-x", "released")],
      },
    });

    expect(groups().map((group) => group.dataset.status)).toEqual([]);
    expect(document.querySelector('[data-status="released"]')).toBeNull();
    expect(document.querySelector('a[href="/tasks/t-x"]')).toBeNull();
  });

  it("counts every card in the group's pill", () => {
    render(StatusTaskList, { props: { fetchState: "ready", items: ITEMS } });

    for (const group of groups()) {
      const pill = group.querySelector<HTMLElement>("[data-count]");
      expect(pill?.textContent?.trim()).toBe(String(cardsOf(group).length));
    }
    // ready holds the plain task, but not the review or the merge.
    const ready = groups().find((group) => group.dataset.status === "ready")!;
    expect(cardsOf(ready)).toHaveLength(1);
  });

  it("hides every task whose kind is not normal, whatever its status", () => {
    render(StatusTaskList, { props: { fetchState: "ready", items: ITEMS } });

    expect(document.querySelector('a[href="/tasks/m-1"]')).toBeNull();
    expect(document.querySelector('a[href="/tasks/r-1"]')).toBeNull();
    for (const group of groups()) {
      for (const card of cardsOf(group)) {
        expect(card.textContent).not.toContain("instant:merge");
        expect(card.textContent).not.toContain("review");
        expect(card.textContent).not.toContain("rework");
      }
    }
  });

  it("hides a done review task and a done instant:merge task from every group", () => {
    // A verdict finishes a review, and a landed merge finishes itself, but
    // neither leaves the top page's status groups: the review's verdict lives
    // on its target's `latest_review`, and a husk card would say nothing the
    // detail page does not already say.
    render(StatusTaskList, {
      props: {
        fetchState: "ready",
        items: [
          summary("r-1", "done", "review", "レビュー: t-done"),
          summary("m-1", "done", "instant:merge", "merge: t-done"),
        ],
        drawnElsewhere: [],
      },
    });

    expect(document.querySelector('a[href="/tasks/r-1"]')).toBeNull();
    expect(document.querySelector('a[href="/tasks/m-1"]')).toBeNull();
    expect(groups()).toHaveLength(0);
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
      "blocked",
    ]);
    const ready = groups().find((group) => group.dataset.status === "ready")!;
    expect(cardsOf(ready).map((card) => card.getAttribute("href"))).toEqual([
      "/tasks/t-ready",
    ]);
    expect(
      ready.querySelector<HTMLElement>("[data-count]")?.textContent?.trim(),
    ).toBe("1");
  });

  it("keeps archived records out even when their retained status is ready", () => {
    render(StatusTaskList, {
      props: {
        fetchState: "ready",
        items: [{ ...summary("old", "ready"), archived: true }],
      },
    });
    expect(screen.queryByRole("link")).toBeNull();
  });

  it("reads a card forest to tree to state: product, then title, then the status badge", () => {
    render(StatusTaskList, { props: { fetchState: "ready", items: ITEMS } });

    const ready = groups().find((group) => group.dataset.status === "ready")!;
    const card = ready.querySelector<HTMLElement>('a[href="/tasks/t-ready"]')!;
    const product = card.querySelector<HTMLElement>(".product")!;
    const title = card.querySelector<HTMLElement>(".name")!;
    const status = card.querySelector<HTMLElement>(".badge")!;
    expect(product.textContent?.trim()).toBe(PRODUCT);
    expect(title.textContent?.trim()).toBe("task t-ready");
    expect(status.textContent?.trim()).toBe("ready");
    // DOM order is reading order: product before title, badge after title.
    expect(
      product.compareDocumentPosition(title) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(
      title.compareDocumentPosition(status) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    // Every card in a status group is a normal task, so the status badge is
    // the only badge it wears.
    expect(card.querySelectorAll(".badge")).toHaveLength(1);
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

  it("says on a ready card which task it is waiting for, until that one lands", () => {
    render(StatusTaskList, {
      props: {
        fetchState: "ready",
        items: [
          {
            ...summary("t-wait", "ready"),
            depends_on: "t-first",
            dependency_status: "wip",
          },
          { ...summary("t-landed", "ready"), depends_on: "t-old" },
          {
            ...summary("t-draft", "draft"),
            depends_on: "t-first",
            dependency_status: "wip",
          },
          summary("t-first", "wip"),
        ],
      },
    });

    const waiting = document.querySelector<HTMLElement>(
      'a[href="/tasks/t-wait"]',
    )!;
    const line = waiting.querySelector<HTMLElement>("[data-waiting-on]")!;
    expect(line.dataset.waitingOn).toBe("t-first");
    expect(line.textContent?.replace(/\s+/g, " ").trim()).toBe(
      "waiting depends_on: t-first",
    );
    expect(waiting.querySelectorAll("a")).toHaveLength(0);
    for (const id of ["t-landed", "t-draft", "t-first"]) {
      const card = document.querySelector<HTMLElement>(
        `a[href="/tasks/${id}"]`,
      )!;
      expect(card.querySelector("[data-waiting-on]"), id).toBeNull();
    }
  });

  it("says who blocked a blocked card, and calls a parked one 保留", () => {
    render(StatusTaskList, {
      props: {
        fetchState: "ready",
        items: [
          { ...summary("t-parked", "blocked"), blocked_by: "operator" },
          { ...summary("t-jam", "blocked"), blocked_by: "worker" },
          { ...summary("t-dep", "blocked"), blocked_by: "system" },
          summary("t-old", "blocked"),
        ],
      },
    });

    const labels = (id: string) =>
      [
        ...document.querySelectorAll<HTMLElement>(
          `a[href="/tasks/${id}"] .badge`,
        ),
      ].map((badge) => badge.textContent?.trim());
    expect(labels("t-parked")).toEqual(["blocked", "保留"]);
    expect(labels("t-jam")).toEqual(["blocked", "worker"]);
    expect(labels("t-dep")).toEqual(["blocked", "system"]);
    // A row the server did not label (pre-migration) wears no second badge.
    expect(labels("t-old")).toEqual(["blocked"]);
    const parked = document.querySelector<HTMLElement>(
      'a[href="/tasks/t-parked"] [data-blocked-by]',
    )!;
    expect(parked.dataset.blockedBy).toBe("operator");
    // The badge is neutral chrome like the status badge: no error tint.
    expect(parked.classList.contains("error-banner")).toBe(false);
  });
});
