import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ClosedTask } from "../lib/api";
import Closed from "./Closed.svelte";

const PRODUCT = "sunny-side/task-server";

function task(over: Partial<ClosedTask> & { id: string }): ClosedTask {
  return {
    title: `task ${over.id}`,
    status: "done",
    product_id: PRODUCT,
    release_tag: null,
    verification: null,
    done_at: "2026-08-15T10:00:00Z",
    closed_at: over.done_at ?? "2026-08-15T10:00:00Z",
    ...over,
  };
}

function jsonResponse(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function stubFetch(handler: () => Response | Promise<Response>) {
  const fetchMock = vi.fn<typeof fetch>(async (input) => {
    if (String(input) === "/api/closed") {
      return handler();
    }
    throw new Error(`unexpected request: ${String(input)}`);
  });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

function region(): HTMLElement {
  const found = document.querySelector<HTMLElement>(
    '[data-region="closed"][data-state]',
  );
  if (!found) {
    throw new Error("closed region with data-state was not found");
  }
  return found;
}

function cards(): HTMLElement[] {
  return [...region().querySelectorAll<HTMLElement>('a[href^="/tasks/"]')];
}

function focusableIn(root: HTMLElement): Element[] {
  return [
    ...root.querySelectorAll(
      'a[href], button, input, select, textarea, [tabindex]:not([tabindex="-1"])',
    ),
  ];
}

// Newest-closed first, exactly as /api/closed already returns it (the
// server owns the sort; this component renders whatever order it receives).
const ROWS: ClosedTask[] = [
  task({
    id: "t-newest",
    title: "最新の完了",
    status: "done",
    done_at: "2026-08-17T09:00:00Z",
    verification: "line one\nline two\nline three",
  }),
  task({
    id: "t-called-off",
    title: "取り下げ",
    status: "cancelled",
    done_at: null,
    closed_at: "2026-08-16T12:00:00Z",
    verification: null,
  }),
  task({
    id: "t-middle",
    title: "中間の完了",
    status: "merged",
    done_at: "2026-08-16T09:00:00Z",
    verification: null,
  }),
  task({
    id: "t-oldest",
    title: "最古の完了、release 済み",
    status: "released",
    release_tag: "v1.0.0",
    done_at: "2026-08-15T09:00:00Z",
    verification: "shipped it",
  }),
];

describe("Closed", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("renders the rows in the order the API returns them", async () => {
    stubFetch(() => jsonResponse(ROWS));

    render(Closed);
    await waitFor(() => expect(region().dataset.state).toBe("success"));

    expect(cards().map((card) => card.getAttribute("href"))).toEqual([
      "/tasks/t-newest",
      "/tasks/t-called-off",
      "/tasks/t-middle",
      "/tasks/t-oldest",
    ]);
    // A called-off task sits in the same list, told apart by its badge and
    // dated by the moment it closed.
    const calledOff = cards()[1];
    expect(calledOff.querySelector(".badge")?.textContent?.trim()).toBe(
      "cancelled",
    );
    expect(calledOff.textContent).toContain("2026-08-16T12:00:00Z");
  });

  it("shows product, status, release tag, done_at, and a two-line excerpt", async () => {
    stubFetch(() => jsonResponse(ROWS));

    render(Closed);
    await waitFor(() => expect(region().dataset.state).toBe("success"));

    const [newest, , , oldest] = cards();
    expect(newest.textContent).toContain("最新の完了");
    expect(newest.textContent).toContain(PRODUCT);
    expect(newest.textContent).toContain("2026-08-17T09:00:00Z");
    expect(newest.querySelector(".badge")?.textContent?.trim()).toBe("done");
    // Only the first two source lines, never the third.
    expect(newest.textContent).toContain("line one");
    expect(newest.textContent).toContain("line two");
    expect(newest.textContent).not.toContain("line three");

    expect(
      [...oldest.querySelectorAll(".badge")].map((badge) =>
        badge.textContent?.trim(),
      ),
    ).toEqual(["released", "v1.0.0"]);
  });

  it("renders no excerpt element for a row with no verification", async () => {
    stubFetch(() => jsonResponse(ROWS));

    render(Closed);
    await waitFor(() => expect(region().dataset.state).toBe("success"));

    const [, middle] = cards();
    expect(middle.querySelector(".excerpt")).toBeNull();
  });

  it("shows exactly one muted line when nothing has finished", async () => {
    stubFetch(() => jsonResponse([]));

    render(Closed);

    await waitFor(() => expect(region().dataset.state).toBe("empty"));
    expect(region().textContent?.trim()).toBe("閉じたタスクがありません");
    expect(cards()).toHaveLength(0);
  });

  it("gives every row exactly one focusable element, the row link itself", async () => {
    stubFetch(() => jsonResponse(ROWS));

    render(Closed);
    await waitFor(() => expect(region().dataset.state).toBe("success"));

    for (const card of cards()) {
      expect(focusableIn(card)).toHaveLength(0);
    }
    expect(focusableIn(region())).toHaveLength(cards().length);
  });

  it("shows the loading state, then a working retry button on failure", async () => {
    let fail = true;
    stubFetch(() =>
      fail ? Promise.reject(new Error("offline")) : jsonResponse(ROWS),
    );

    render(Closed);
    expect(region().dataset.state).toBe("loading");

    await waitFor(() => expect(region().dataset.state).toBe("error"));
    const retry = screen.getByRole("button", { name: "再試行" });

    fail = false;
    await fireEvent.click(retry);

    await waitFor(() => expect(region().dataset.state).toBe("success"));
    expect(cards()).toHaveLength(4);
  });
});
