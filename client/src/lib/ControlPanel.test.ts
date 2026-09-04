import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  ControlPlane,
  PendingMerge,
  PendingRelease,
  TaskSummary,
} from "./api";
import ControlPanel from "./ControlPanel.svelte";

const PRODUCT = "sunny-side/task-server";

function summary(
  id: string,
  status: string,
  kind = "normal",
  productId = PRODUCT,
): TaskSummary {
  return {
    id,
    title: `task ${id}`,
    status,
    kind,
    product_id: productId,
    priority: 0,
    updated_at: "2026-08-15T12:00:00Z",
  };
}

// A pending merge carries its own stop reason; a running one has none.
function merge(id: string, verification: string | null = null): PendingMerge {
  return {
    ...summary(
      id,
      verification === null ? "ready" : "blocked",
      "instant:merge",
    ),
    verification,
  };
}

// A pending release is the same shape, plus how far it steps the version.
function release(
  id: string,
  level: PendingRelease["release_level"] = "patch",
  verification: string | null = null,
  productId = PRODUCT,
): PendingRelease {
  return {
    ...summary(
      id,
      verification === null ? "ready" : "blocked",
      "instant:release",
      productId,
    ),
    release_level: level,
    verification,
  };
}

function plane(over: Partial<ControlPlane> = {}): ControlPlane {
  return {
    mergeable: [],
    pending_merges: [],
    pending_releases: [],
    pending_reviews: [],
    unreviewed: [],
    releasable: [],
    stuck: [],
    ...over,
  };
}

function panel(): HTMLElement {
  const found = document.querySelector<HTMLElement>('[data-region="control"]');
  if (!found) {
    throw new Error("the control panel was not found");
  }
  return found;
}

function block(name: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`[data-block="${name}"]`);
}

describe("ControlPanel", () => {
  afterEach(cleanup);

  it("holds no control at all: the top page asks the human for nothing", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({
          mergeable: [summary("t-done", "done")],
          pending_merges: [merge("m-1")],
          pending_releases: [release("release:t-1")],
          pending_reviews: [summary("r-1", "ready", "review")],
          releasable: [{ product_id: "sunny-side/other", task_count: 1 }],
        }),
      },
    });

    expect(screen.queryAllByRole("button")).toHaveLength(0);
    expect(panel().querySelector(".primary")).toBeNull();
    for (const word of ["release", "merge"]) {
      for (const one of panel().querySelectorAll("button")) {
        expect(one.textContent?.toLowerCase()).not.toContain(word);
      }
    }
  });

  it("says so in one muted line when the server is carrying nothing", () => {
    render(ControlPanel, { props: { fetchState: "ready", plane: plane() } });

    expect(panel().dataset.state).toBe("empty");
    expect(block("idle")?.textContent).toContain("運んでいるものはありません");
    expect(screen.queryAllByRole("button")).toHaveLength(0);
  });

  it("renders the review queue with its count, and not at all when empty", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({
          pending_reviews: [
            summary("r-1", "ready", "review"),
            summary("r-2", "wip", "review"),
          ],
        }),
      },
    });

    const queue = block("reviews")!;
    expect(queue.textContent).toContain("review 待ち");
    const cards = [
      ...queue.querySelectorAll<HTMLElement>('a[href^="/tasks/"]'),
    ];
    expect(cards.map((card) => card.getAttribute("href"))).toEqual([
      "/tasks/r-1",
      "/tasks/r-2",
    ]);
    expect(
      queue.querySelector<HTMLElement>("[data-count]")?.textContent?.trim(),
    ).toBe(String(cards.length));
    // A readout has no status heading over it, so its cards carry the badge.
    expect(
      cards.map((card) => card.querySelector(".badge")?.textContent?.trim()),
    ).toEqual(["ready", "wip"]);

    cleanup();
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({ pending_merges: [merge("m-1")] }),
      },
    });

    expect(block("reviews")).toBeNull();
    expect(document.body.textContent).not.toContain("review 待ち");
  });

  it("draws the releases like the merge trains: product, level, status, and a reason when stopped", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({
          pending_releases: [
            release("release:t-1", "minor"),
            release(
              "release:t-9",
              "patch",
              "bump-tag: the tag already exists",
              "sunny-side/other",
            ),
          ],
        }),
      },
    });

    const readout = block("releases")!;
    expect(
      readout.querySelector<HTMLElement>("[data-count]")?.textContent?.trim(),
    ).toBe("2");
    const cards = [
      ...readout.querySelectorAll<HTMLElement>('a[href^="/tasks/"]'),
    ];
    expect(cards.map((card) => card.getAttribute("href"))).toEqual([
      "/tasks/release:t-1",
      "/tasks/release:t-9",
    ]);
    expect(cards[0].textContent).toContain(PRODUCT);
    expect(cards[0].querySelector("[data-level]")?.textContent?.trim()).toBe(
      "minor",
    );
    expect(
      [...cards[0].querySelectorAll(".badge")].map((badge) =>
        badge.textContent?.trim(),
      ),
    ).toEqual(["minor", "ready"]);
    expect(cards[0].querySelector("[data-reason]")).toBeNull();

    const stopped = cards[1];
    expect(stopped.textContent).toContain("sunny-side/other");
    expect(stopped.querySelector("[data-reason]")?.textContent).toContain(
      "the tag already exists",
    );
    // Neutral: a tag that could not be cut is not a failure of the app.
    expect(stopped.classList.contains("error-banner")).toBe(false);

    cleanup();
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({ pending_merges: [merge("m-1")] }),
      },
    });
    expect(block("releases")).toBeNull();
  });

  it("gives every readout card exactly one focus stop, the link itself", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({
          pending_reviews: [summary("r-1", "ready", "review")],
          pending_releases: [release("release:t-1")],
          unreviewed: [summary("t-done", "done")],
        }),
      },
    });

    const cards = [
      ...panel().querySelectorAll<HTMLElement>('a[href^="/tasks/"]'),
    ];
    expect(cards).toHaveLength(3);
    for (const card of cards) {
      expect(
        card.querySelectorAll(
          'a[href], button, input, select, textarea, [tabindex]:not([tabindex="-1"])',
        ),
      ).toHaveLength(0);
      expect(card.hasAttribute("tabindex")).toBe(false);
    }
  });

  it("frames stranded work as a standing state, never as an alert", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({
          unreviewed: [summary("t-done", "done")],
          mergeable: [
            summary("t-app-1", "approved"),
            summary("t-app-2", "approved"),
          ],
          releasable: [
            { product_id: PRODUCT, task_count: 2 },
            { product_id: "sunny-side/other", task_count: 1 },
          ],
        }),
      },
    });

    const stranded = block("reconciliation")!;
    expect(stranded.getAttribute("role")).toBe("status");
    expect(stranded.getAttribute("role")).not.toBe("alert");
    expect(stranded.classList.contains("info-banner")).toBe(true);
    expect(stranded.classList.contains("error-banner")).toBe(false);
    expect(stranded.querySelector(".danger, .error-banner")).toBeNull();

    const sets = [...stranded.querySelectorAll<HTMLElement>("[data-readout]")];
    expect(sets.map((set) => set.dataset.readout)).toEqual([
      "unreviewed",
      "mergeable",
      "releasable",
    ]);
    for (const set of sets.slice(0, 2)) {
      const cards = set.querySelectorAll('a[href^="/tasks/"]');
      expect(
        set.querySelector<HTMLElement>("[data-count]")?.textContent?.trim(),
      ).toBe(String(cards.length));
      // The tint frames the fact, never the tasks: ordinary cards inside.
      for (const card of cards) {
        expect(card.classList.contains("card")).toBe(true);
        expect(card.classList.contains("error-banner")).toBe(false);
        // The ordinary list card: product first, then the title.
        expect(card.querySelector(".product-first")).not.toBeNull();
        expect(card.querySelector(".name")).not.toBeNull();
      }
    }
    // Stranded releases are counted per product, with the work each carries.
    const products = sets[2];
    expect(products.textContent).toContain(
      "release が発行されていない product",
    );
    const rows = [...products.querySelectorAll<HTMLElement>("[data-product]")];
    expect(rows.map((row) => row.dataset.product)).toEqual([
      PRODUCT,
      "sunny-side/other",
    ]);
    expect(
      rows.map((row) => row.querySelector("[data-count]")?.textContent?.trim()),
    ).toEqual(["2", "1"]);
    expect(products.querySelector("button")).toBeNull();
  });

  it("draws only the non-empty part of reconciliation, and none of it when healthy", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({ unreviewed: [summary("t-done", "done")] }),
      },
    });

    const stranded = block("reconciliation")!;
    expect(
      [...stranded.querySelectorAll<HTMLElement>("[data-readout]")].map(
        (set) => set.dataset.readout,
      ),
    ).toEqual(["unreviewed"]);

    cleanup();
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({ pending_reviews: [summary("r-1", "ready", "review")] }),
      },
    });

    expect(block("reconciliation")).toBeNull();
  });

  it("shows a spinner while loading and offers a retry on failure", async () => {
    render(ControlPanel, { props: { fetchState: "loading" } });

    expect(panel().dataset.state).toBe("loading");
    expect(screen.queryByRole("button")).toBeNull();
    expect(panel().querySelector(".spinner")).not.toBeNull();

    cleanup();
    const onretry = vi.fn();
    render(ControlPanel, { props: { fetchState: "error", onretry } });

    expect(panel().dataset.state).toBe("error");
    expect(panel().querySelector(".state.error")?.textContent).toContain(
      "読み込みに失敗しました",
    );
    await fireEvent.click(screen.getByRole("button", { name: "再試行" }));
    expect(onretry).toHaveBeenCalledTimes(1);
  });

  it("keeps the panel's blocks in pipeline order", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({
          pending_reviews: [summary("r-1", "ready", "review")],
          pending_merges: [merge("m-1")],
          pending_releases: [release("release:t-1")],
          unreviewed: [summary("t-done", "done")],
        }),
      },
    });

    expect(
      [...panel().querySelectorAll<HTMLElement>("[data-block]")].map(
        (one) => one.dataset.block,
      ),
    ).toEqual(["reviews", "trains", "releases", "reconciliation"]);
  });

  it("states stuck work per reason, in plain words, as ordinary task cards", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({
          stuck: [
            {
              task_id: "t-old",
              kind: "normal",
              status: "ready",
              since: "2026-09-03T10:00:00Z",
              reason: "unclaimed",
            },
            {
              task_id: "t-held",
              kind: "normal",
              status: "blocked",
              since: "2026-09-03T08:00:00Z",
              reason: "blocked",
            },
            {
              task_id: "release:t-9",
              kind: "instant:release",
              status: "wip",
              since: "2026-09-03T09:00:00Z",
              reason: "release-stalled",
            },
          ],
        }),
        tasks: [
          summary("t-old", "ready"),
          { ...summary("t-held", "blocked"), blocked_by: "worker" },
        ],
      },
    });

    const stranded = block("reconciliation")!;
    expect(stranded.getAttribute("role")).toBe("status");
    const set = stranded.querySelector<HTMLElement>('[data-readout="stuck"]')!;
    expect(set.textContent).not.toContain("動いていない task");

    // One caption per reason, in the server's order, each with its one-line
    // note under it and its own count.
    const groups = [...set.querySelectorAll<HTMLElement>("[data-reason]")];
    expect(groups.map((group) => group.dataset.reason)).toEqual([
      "unclaimed",
      "blocked",
      "release-stalled",
    ]);
    const caption = (group: HTMLElement) =>
      group.querySelector(".caption")?.textContent?.trim();
    const note = (group: HTMLElement) =>
      group.querySelector(".note")?.textContent?.trim();
    expect(caption(groups[0])).toBe("長時間 claim されていない");
    expect(caption(groups[1])).toBe("長時間 blocked 状態");
    expect(note(groups[1])).toBe("追加の議論が必要でしょうか。ご確認ください");
    expect(caption(groups[2])).toBe("release が進んでいない");
    for (const group of groups) {
      expect(note(group)).toBeTruthy();
      expect(group.querySelector("[data-count]")?.textContent?.trim()).toBe(
        String(group.querySelectorAll('a[href^="/tasks/"]').length),
      );
    }

    // The rows are the ordinary task card: product → title → status / kind,
    // the whole card one link, nothing about the reason on it.
    const rows = set.querySelectorAll<HTMLAnchorElement>('a[href^="/tasks/"]');
    expect(rows).toHaveLength(3);
    const old = rows[0];
    expect(old.getAttribute("href")).toBe("/tasks/t-old");
    expect(old.classList.contains("card")).toBe(true);
    expect(old.querySelector(".product-first")?.textContent).toBe(PRODUCT);
    expect(old.querySelector(".name")?.textContent).toBe("task t-old");
    expect(
      [...old.querySelectorAll(".badge")].map((b) => b.textContent?.trim()),
    ).toEqual(["ready"]);
    expect(old.textContent).not.toContain("unclaimed");
    expect(old.textContent).not.toContain("2026-09-03");
    expect(
      [...rows[1].querySelectorAll(".badge")].map((b) => b.textContent?.trim()),
    ).toEqual(["blocked", "worker"]);
    // A row the summaries do not know keeps its id as the title, its status
    // and kind from the server, and no invented product.
    const orphan = rows[2];
    expect(orphan.getAttribute("href")).toBe("/tasks/release:t-9");
    expect(orphan.querySelector(".name")?.textContent).toBe("release:t-9");
    expect(orphan.querySelector(".product-first")?.textContent ?? "").toBe("");
    expect(
      [...orphan.querySelectorAll(".badge")].map((b) => b.textContent?.trim()),
    ).toEqual(["wip", "instant:release"]);
    for (const row of rows) {
      expect(row.querySelectorAll("a, button, [tabindex]")).toHaveLength(0);
      expect(row.querySelector(".card")).toBeNull();
    }
    expect(set.querySelector("[data-task]")).toBeNull();
    expect(stranded.querySelectorAll("button")).toHaveLength(0);
    expect(stranded.querySelector(".danger")).toBeNull();
  });

  it("draws no stuck readout while the server reports none", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({ unreviewed: [summary("t-done", "done")] }),
      },
    });
    expect(document.querySelector('[data-readout="stuck"]')).toBeNull();
  });
});
