import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import Home from "./Home.svelte";

type Summary = {
  id: string;
  title: string;
  status: string;
  kind: string;
  product_id: string;
  priority: number;
  updated_at: string;
};

// A pending merge is a summary plus the reason it stopped, which is how
// /api/control sends it: the queue and the jam arrive together.
type PendingMerge = Summary & { verification: string | null };

// A pending release is the same shape plus how far it steps the version.
type PendingRelease = PendingMerge & {
  release_level: "patch" | "minor" | "major";
};

type Plane = {
  mergeable: Summary[];
  pending_merges: PendingMerge[];
  pending_releases: PendingRelease[];
  pending_reviews: Summary[];
  unreviewed: Summary[];
  releasable: { product_id: string; task_count: number }[];
  stuck: unknown[];
};

const PRODUCT = "sunny-side/task-server";

function summary(
  id: string,
  status: string,
  kind = "normal",
  title = `task ${id}`,
  productId = PRODUCT,
): Summary {
  return {
    id,
    title,
    status,
    kind,
    product_id: productId,
    priority: 0,
    updated_at: "2026-08-15T12:00:00Z",
  };
}

function pendingMerge(
  id: string,
  status = "ready",
  verification: string | null = null,
  productId = PRODUCT,
): PendingMerge {
  return {
    ...summary(id, status, "instant:merge", `merge ${id}`, productId),
    verification,
  };
}

// Deliberately unordered, and carrying the two things the top page must never
// show: a released task and an instant:merge task (DESIGN.md, Task list).
const TASKS: Summary[] = [
  summary("t-merged", "merged"),
  summary("t-draft", "draft"),
  summary("t-blocked", "blocked"),
  summary("t-ready-1", "ready", "normal", "テーマ切替"),
  summary("t-released", "released"),
  summary("t-wip", "wip"),
  summary("t-ready-2", "ready"),
  summary("m-1", "ready", "instant:merge"),
  summary("t-done", "done"),
];

const EMPTY_PLANE: Plane = {
  mergeable: [],
  pending_merges: [],
  pending_releases: [],
  pending_reviews: [],
  unreviewed: [],
  releasable: [],
  stuck: [],
};

function plane(over: Partial<Plane> = {}): Plane {
  return { ...EMPTY_PLANE, ...over };
}

function jsonResponse(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { "content-type": "application/json" },
  });
}

interface Scenario {
  control?: () => Response | Promise<Response>;
  tasks?: () => Response | Promise<Response>;
}

function stubFetch(scenario: Scenario) {
  const fetchMock = vi.fn<typeof fetch>(async (input, init) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    if (url === "/api/control" && method === "GET") {
      return (scenario.control ?? (() => jsonResponse(EMPTY_PLANE)))();
    }
    if (url === "/api/tasks" && method === "GET") {
      return (scenario.tasks ?? (() => jsonResponse(TASKS)))();
    }
    // The top page reads the queue and the list, and nothing card by card.
    // A per-card request here is the extra round trip this page must not make.
    if (url.startsWith("/api/tasks/") && method === "GET") {
      throw new Error(
        `unexpected task card request: ${decodeURIComponent(url.slice("/api/tasks/".length))}`,
      );
    }
    throw new Error(`unexpected request: ${method} ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

function callsTo(
  fetchMock: ReturnType<typeof stubFetch>,
  url: string,
  method = "GET",
) {
  return fetchMock.mock.calls.filter(
    ([input, init]) =>
      String(input) === url && (init?.method ?? "GET") === method,
  );
}

function writes(fetchMock: ReturnType<typeof stubFetch>) {
  return fetchMock.mock.calls.filter(
    ([, init]) => (init?.method ?? "GET") !== "GET",
  );
}

function region(name: "control" | "tasks"): HTMLElement {
  const found = document.querySelector<HTMLElement>(
    `[data-region="${name}"][data-state]`,
  );
  if (!found) {
    throw new Error(`region ${name} with data-state was not found`);
  }
  return found;
}

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: state,
  });
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("Home", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("only reads on load and keeps creation available with legacy queues", async () => {
    const fetchMock = stubFetch({
      control: () =>
        jsonResponse(
          plane({
            pending_merges: [pendingMerge("m-1")],
            pending_reviews: [summary("r-1", "ready", "review")],
          }),
        ),
    });
    render(Home);
    await waitFor(() => expect(region("tasks").dataset.state).toBe("success"));
    expect(screen.getByRole("button", { name: "新規タスク" })).toBeTruthy();
    expect(region("control").dataset.state).toBe("empty");
    expect(writes(fetchMock)).toHaveLength(0);
    expect(
      [...region("tasks").querySelectorAll<HTMLElement>("[data-status]")].map(
        (g) => g.dataset.status,
      ),
    ).toEqual(["draft", "ready", "wip", "blocked"]);
  });

  it("shows a stopped task once in the execution readout", async () => {
    stubFetch({
      control: () =>
        jsonResponse(
          plane({
            stuck: [
              {
                task_id: "t-blocked",
                status: "blocked",
                kind: "normal",
                since: "2026-09-05",
                reason: "blocked",
              },
            ],
          }),
        ),
    });
    render(Home);
    await waitFor(() =>
      expect(region("control").dataset.state).toBe("success"),
    );
    await waitFor(() => expect(region("tasks").dataset.state).toBe("success"));
    expect(
      document.querySelectorAll('a[href="/tasks/t-blocked"]'),
    ).toHaveLength(1);
    expect(
      region("tasks").querySelector('a[href="/tasks/t-blocked"]'),
    ).toBeNull();
  });

  it("shows the empty state for an empty list", async () => {
    stubFetch({ tasks: () => jsonResponse([]) });

    render(Home);

    await waitFor(() => expect(region("tasks").dataset.state).toBe("empty"));
  });

  it("recovers each region through its own retry button", async () => {
    let controlFails = true;
    let tasksFail = true;
    stubFetch({
      control: () =>
        controlFails
          ? Promise.reject(new Error("offline"))
          : jsonResponse(EMPTY_PLANE),
      tasks: () =>
        tasksFail ? Promise.reject(new Error("offline")) : jsonResponse(TASKS),
    });

    render(Home);
    await waitFor(() => expect(region("control").dataset.state).toBe("error"));
    await waitFor(() => expect(region("tasks").dataset.state).toBe("error"));

    controlFails = false;
    await fireEvent.click(
      region("control").querySelector<HTMLButtonElement>("button")!,
    );
    await waitFor(() => expect(region("control").dataset.state).toBe("empty"));
    expect(region("tasks").dataset.state).toBe("error");

    tasksFail = false;
    await fireEvent.click(
      region("tasks").querySelector<HTMLButtonElement>("button")!,
    );
    await waitFor(() => expect(region("tasks").dataset.state).toBe("success"));
  });

  it("reloads both regions when the tab becomes visible again", async () => {
    const fetchMock = stubFetch({});

    render(Home);
    await waitFor(() => expect(region("tasks").dataset.state).toBe("success"));

    setVisibility("hidden");
    expect(callsTo(fetchMock, "/api/tasks")).toHaveLength(1);

    setVisibility("visible");
    await waitFor(() =>
      expect(callsTo(fetchMock, "/api/tasks")).toHaveLength(2),
    );
    expect(callsTo(fetchMock, "/api/control")).toHaveLength(2);
  });

  it("reloads on the recurring interval while visible, and stops after unmount", async () => {
    vi.useFakeTimers();
    try {
      const fetchMock = stubFetch({});

      const { unmount } = render(Home);
      await vi.waitFor(() =>
        expect(region("tasks").dataset.state).toBe("success"),
      );
      expect(callsTo(fetchMock, "/api/tasks")).toHaveLength(1);

      await vi.advanceTimersByTimeAsync(30_000);
      expect(callsTo(fetchMock, "/api/tasks")).toHaveLength(2);
      expect(callsTo(fetchMock, "/api/control")).toHaveLength(2);

      unmount();
      await vi.advanceTimersByTimeAsync(60_000);
      expect(callsTo(fetchMock, "/api/tasks")).toHaveLength(2);
      expect(callsTo(fetchMock, "/api/control")).toHaveLength(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps the drawn panel and list when a background reload fails", async () => {
    let fail = false;
    const fetchMock = stubFetch({
      control: () =>
        fail
          ? Promise.reject(new Error("offline"))
          : jsonResponse(
              plane({
                stuck: [
                  {
                    task_id: "t-blocked",
                    status: "blocked",
                    kind: "normal",
                    since: "2026-09-05",
                    reason: "blocked",
                  },
                ],
              }),
            ),
      tasks: () =>
        fail ? Promise.reject(new Error("offline")) : jsonResponse(TASKS),
    });

    render(Home);
    await waitFor(() =>
      expect(region("control").dataset.state).toBe("success"),
    );
    await waitFor(() => expect(region("tasks").dataset.state).toBe("success"));
    expect(
      screen.getByRole("link", { name: /テーマ切替/ }).getAttribute("href"),
    ).toBe("/tasks/t-ready-1");

    fail = true;
    setVisibility("hidden");
    setVisibility("visible");
    await waitFor(() =>
      expect(callsTo(fetchMock, "/api/control")).toHaveLength(2),
    );
    await waitFor(() =>
      expect(callsTo(fetchMock, "/api/tasks")).toHaveLength(2),
    );

    // `callsTo` above only proves the requests were sent, not that their
    // rejections have been caught yet — let that settle before reading
    // state, or this assertion would just race ahead of the update.
    await new Promise((resolve) => setTimeout(resolve, 0));

    // The failed background reload never swaps the success state for the
    // error one: the panel and the list stay exactly as they were drawn
    // (DESIGN.md, Do's and Don'ts).
    expect(region("control").dataset.state).toBe("success");
    expect(region("tasks").dataset.state).toBe("success");
    expect(
      screen.getByRole("link", { name: /テーマ切替/ }).getAttribute("href"),
    ).toBe("/tasks/t-ready-1");
  });
});
