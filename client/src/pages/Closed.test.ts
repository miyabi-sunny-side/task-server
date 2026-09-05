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

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: state,
  });
  document.dispatchEvent(new Event("visibilitychange"));
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
    summary: "依存の順序を claim 側へ移した。全 test 緑。",
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
    verification: "x".repeat(100) + "\nsecond line of the log",
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

  it("reads product, title, summary, then the tail; the log never reaches the list", async () => {
    stubFetch(() => jsonResponse(ROWS));

    render(Closed);
    await waitFor(() => expect(region().dataset.state).toBe("success"));

    const [newest, , , oldest] = cards();
    // Forest, not trees: product → title → summary → when / status / tag.
    const product = newest.querySelector(".product-first")!;
    const name = newest.querySelector(".name")!;
    const summary = newest.querySelector(".summary")!;
    expect(product.textContent).toBe(PRODUCT);
    expect(name.textContent).toBe("最新の完了");
    expect(summary.textContent).toBe(
      "依存の順序を claim 側へ移した。全 test 緑。",
    );
    expect(
      product.compareDocumentPosition(name) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(
      name.compareDocumentPosition(summary) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(newest.textContent).toContain("2026-08-17T09:00:00Z");
    expect(newest.querySelector(".badge")?.textContent?.trim()).toBe("done");
    // With a summary the log stays off the list entirely.
    expect(newest.textContent).not.toContain("line one");
    expect(newest.querySelector(".excerpt")).toBeNull();

    // Without a summary: the log's first line, cut at 80 characters, nothing more.
    const fallback = oldest.querySelector(".summary")!;
    expect(fallback.textContent).toBe("x".repeat(80));
    expect(oldest.textContent).not.toContain("second line");
    expect(oldest.textContent).not.toContain("x".repeat(81));
    expect(
      [...oldest.querySelectorAll(".badge")].map((badge) =>
        badge.textContent?.trim(),
      ),
    ).toEqual(["released", "v1.0.0"]);
  });

  it("renders no summary element for a row with neither summary nor verification", async () => {
    stubFetch(() => jsonResponse(ROWS));

    render(Closed);
    await waitFor(() => expect(region().dataset.state).toBe("success"));

    const [, middle] = cards();
    expect(middle.querySelector(".summary")).toBeNull();
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

  it("reloads when the tab becomes visible again and on the interval, and stops after unmount", async () => {
    vi.useFakeTimers();
    try {
      const fetchMock = stubFetch(() => jsonResponse([task({ id: "t-1" })]));
      const { unmount } = render(Closed);
      await vi.waitFor(() => expect(region().dataset.state).toBe("success"));
      expect(fetchMock).toHaveBeenCalledTimes(1);

      setVisibility("hidden");
      await vi.advanceTimersByTimeAsync(60_000);
      expect(fetchMock).toHaveBeenCalledTimes(1);

      setVisibility("visible");
      await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));

      await vi.advanceTimersByTimeAsync(30_000);
      expect(fetchMock).toHaveBeenCalledTimes(3);

      unmount();
      await vi.advanceTimersByTimeAsync(60_000);
      setVisibility("hidden");
      setVisibility("visible");
      expect(fetchMock).toHaveBeenCalledTimes(3);
    } finally {
      vi.useRealTimers();
      setVisibility("visible");
    }
  });

  it("keeps the drawn rows while a background reload is in flight or fails", async () => {
    let mode: "ok" | "slow" | "fail" = "ok";
    let release: (() => void) | undefined;
    const fetchMock = stubFetch(() => {
      if (mode === "fail") return Promise.reject(new Error("offline"));
      if (mode === "slow") {
        return new Promise<Response>((resolve) => {
          release = () => resolve(jsonResponse([task({ id: "t-2" })]));
        });
      }
      return jsonResponse([task({ id: "t-1" })]);
    });

    render(Closed);
    await waitFor(() => expect(region().dataset.state).toBe("success"));
    expect(cards().map((card) => card.getAttribute("href"))).toEqual([
      "/tasks/t-1",
    ]);

    // In flight: the rows stay, no spinner takes their place.
    mode = "slow";
    setVisibility("hidden");
    setVisibility("visible");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(region().dataset.state).toBe("success");
    expect(cards().map((card) => card.getAttribute("href"))).toEqual([
      "/tasks/t-1",
    ]);
    release!();
    await waitFor(() =>
      expect(cards().map((card) => card.getAttribute("href"))).toEqual([
        "/tasks/t-2",
      ]),
    );

    // Failed: the rows stay and the state never turns to error.
    mode = "fail";
    setVisibility("hidden");
    setVisibility("visible");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(3));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(region().dataset.state).toBe("success");
    expect(cards().map((card) => card.getAttribute("href"))).toEqual([
      "/tasks/t-2",
    ]);
    expect(screen.queryByRole("button", { name: "再試行" })).toBeNull();
  });
});

it("keeps archived legacy tasks and dropped records readable in history", async () => {
  stubFetch(() =>
    jsonResponse([
      task({ id: "old-review", status: "ready", archived: true }),
      task({ id: "discarded", status: "dropped" }),
    ]),
  );
  render(Closed);
  await screen.findByText("履歴");
  expect(
    screen.getByRole("link", { name: /old-review/ }).getAttribute("href"),
  ).toBe("/tasks/old-review");
  expect(screen.getByRole("link", { name: /discarded/ }).textContent).toContain(
    "dropped",
  );
  cleanup();
  vi.unstubAllGlobals();
});
