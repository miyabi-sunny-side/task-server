import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ControlPlane, PendingMerge, TaskSummary } from "./api";
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

function plane(over: Partial<ControlPlane> = {}): ControlPlane {
  return {
    mergeable: [],
    pending_merges: [],
    pending_reviews: [],
    unreviewed: [],
    releasable: [],
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

function noteOf(button: HTMLElement): HTMLElement | null {
  return document.getElementById(
    String(button.getAttribute("aria-describedby")),
  );
}

function releaseButton(): HTMLElement {
  return screen.getByRole("button", { name: "release" });
}

function block(name: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`[data-block="${name}"]`);
}

describe("ControlPanel", () => {
  afterEach(cleanup);

  it("gives the control row one button, release, and the page's only accent", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({
          releasable: [
            { product_id: PRODUCT, task_count: 2 },
            { product_id: "sunny-side/other", task_count: 1 },
          ],
          mergeable: [summary("t-done", "done")],
          pending_merges: [merge("m-1")],
          pending_reviews: [summary("r-1", "ready", "review")],
        }),
      },
    });

    const row = block("control")!;
    expect(row.querySelectorAll("button")).toHaveLength(1);
    expect(screen.getAllByRole("button")).toHaveLength(1);
    const release = releaseButton();
    expect(release.classList.contains("primary")).toBe(true);
    expect(
      [...panel().querySelectorAll(".primary")].map((one) =>
        one.textContent?.trim(),
      ),
    ).toEqual(["release"]);

    const note = noteOf(release);
    expect(note?.classList.contains("pill")).toBe(true);
    expect(note?.textContent?.trim()).toBe("2");
  });

  it("issues no merge: no control on the panel is named for merge", () => {
    const onrelease = vi.fn();
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({
          mergeable: [summary("t-a", "done"), summary("t-b", "done")],
          pending_merges: [merge("m-1")],
        }),
        onrelease,
      },
    });

    const buttons = screen.getAllByRole("button");
    expect(buttons.map((one) => one.textContent?.trim())).toEqual(["release"]);
    for (const one of buttons) {
      expect(one.textContent?.toLowerCase()).not.toContain("merge");
    }
    expect(document.getElementById("control-merge")).toBeNull();
  });

  it("drops the accent to the default treatment and names why when idle", async () => {
    const onrelease = vi.fn();
    render(ControlPanel, {
      props: { fetchState: "ready", plane: plane(), onrelease },
    });

    expect(panel().dataset.state).toBe("empty");
    const release = releaseButton();
    expect(release.classList.contains("primary")).toBe(false);
    expect(release.getAttribute("aria-disabled")).toBe("true");
    // Still reachable and still explained: opacity alone never carries a reason.
    expect(release.hasAttribute("disabled")).toBe(false);
    expect(noteOf(release)?.textContent?.trim()).toBe(
      "release 可能な product はありません",
    );

    await fireEvent.click(release);
    expect(onrelease).not.toHaveBeenCalled();
  });

  it("opens the release flow only while a product is releasable", async () => {
    const onrelease = vi.fn();
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({ releasable: [{ product_id: PRODUCT, task_count: 1 }] }),
        onrelease,
      },
    });

    await fireEvent.click(releaseButton());
    expect(onrelease).toHaveBeenCalledTimes(1);
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
    render(ControlPanel, { props: { fetchState: "ready", plane: plane() } });

    expect(block("reviews")).toBeNull();
    expect(document.body.textContent).not.toContain("review 待ち");
    // The control survives what the readout does not.
    expect(releaseButton()).toBeTruthy();
  });

  it("gives every readout card exactly one focus stop, the link itself", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({
          pending_reviews: [summary("r-1", "ready", "review")],
          unreviewed: [summary("t-done", "done")],
        }),
      },
    });

    const cards = [
      ...panel().querySelectorAll<HTMLElement>('a[href^="/tasks/"]'),
    ];
    expect(cards).toHaveLength(2);
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
        }),
      },
    });

    const stranded = block("reconciliation")!;
    expect(stranded.getAttribute("role")).toBe("status");
    expect(stranded.getAttribute("role")).not.toBe("alert");
    expect(stranded.classList.contains("error-banner")).toBe(true);

    const sets = [...stranded.querySelectorAll<HTMLElement>("[data-readout]")];
    expect(sets.map((set) => set.dataset.readout)).toEqual([
      "unreviewed",
      "mergeable",
    ]);
    for (const set of sets) {
      const cards = set.querySelectorAll('a[href^="/tasks/"]');
      expect(
        set.querySelector<HTMLElement>("[data-count]")?.textContent?.trim(),
      ).toBe(String(cards.length));
      // The tint frames the fact, never the tasks: ordinary cards inside.
      for (const card of cards) {
        expect(card.classList.contains("card")).toBe(true);
        expect(card.classList.contains("error-banner")).toBe(false);
      }
    }
  });

  it("draws only the non-empty half of reconciliation, and none of it when healthy", () => {
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

  it("hides the control row while loading and offers a retry on failure", async () => {
    render(ControlPanel, { props: { fetchState: "loading" } });

    expect(panel().dataset.state).toBe("loading");
    expect(screen.queryByRole("button")).toBeNull();
    expect(block("control")).toBeNull();
    expect(panel().querySelector(".spinner")).not.toBeNull();

    cleanup();
    const onretry = vi.fn();
    render(ControlPanel, { props: { fetchState: "error", onretry } });

    expect(panel().dataset.state).toBe("error");
    expect(screen.queryByRole("button", { name: "release" })).toBeNull();
    expect(panel().querySelector(".state.error")?.textContent).toContain(
      "読み込みに失敗しました",
    );
    await fireEvent.click(screen.getByRole("button", { name: "再試行" }));
    expect(onretry).toHaveBeenCalledTimes(1);
  });

  it("reports an action's outcome in one live region under the control row", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane(),
        result: { kind: "error", message: "release を拒否されました" },
      },
    });

    const line = block("result")!;
    expect(line.getAttribute("aria-live")).toBe("polite");
    expect(line.querySelector('[role="alert"]')?.textContent).toContain(
      "release を拒否されました",
    );
    // Blocks are ordered: what you can do, then what it did, then the readouts.
    expect(
      block("control")!.compareDocumentPosition(line) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("keeps the panel's blocks in pipeline order", () => {
    render(ControlPanel, {
      props: {
        fetchState: "ready",
        plane: plane({
          releasable: [{ product_id: PRODUCT, task_count: 1 }],
          pending_reviews: [summary("r-1", "ready", "review")],
          pending_merges: [merge("m-1")],
          unreviewed: [summary("t-done", "done")],
        }),
      },
    });

    expect(
      [...panel().querySelectorAll<HTMLElement>("[data-block]")].map(
        (one) => one.dataset.block,
      ),
    ).toEqual(["control", "result", "reviews", "trains", "reconciliation"]);
  });
});
