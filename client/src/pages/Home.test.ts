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

  it("the two regions load and fail independently", async () => {
    // The list request fails; the control panel must still draw.
    stubFetch({
      control: () =>
        jsonResponse(
          plane({
            pending_releases: [
              {
                ...pendingMerge("release:t-1", "ready", null),
                kind: "instant:release",
                release_level: "patch",
              },
            ],
          }),
        ),
      tasks: () => Promise.reject(new Error("offline")),
    });

    render(Home);
    expect(region("control").dataset.state).toBe("loading");
    expect(region("tasks").dataset.state).toBe("loading");

    await waitFor(() =>
      expect(region("control").dataset.state).toBe("success"),
    );
    await waitFor(() => expect(region("tasks").dataset.state).toBe("error"));
    expect(
      region("control").querySelector('a[href="/tasks/release:t-1"]'),
    ).not.toBeNull();

    cleanup();
    vi.unstubAllGlobals();

    // Now the other way round: the control request fails, the list renders.
    stubFetch({
      control: () => Promise.reject(new Error("offline")),
      tasks: () => jsonResponse(TASKS),
    });

    render(Home);
    await waitFor(() => expect(region("tasks").dataset.state).toBe("success"));
    await waitFor(() => expect(region("control").dataset.state).toBe("error"));

    const list = region("tasks");
    const groups = [...list.querySelectorAll<HTMLElement>("[data-status]")];
    expect(groups.map((group) => group.dataset.status)).toEqual([
      "draft",
      "ready",
      "wip",
      "done",
      "merged",
      "blocked",
    ]);
    for (const group of groups) {
      const pill = group.querySelector<HTMLElement>("[data-count]");
      const cards = group.querySelectorAll('a[href^="/tasks/"]');
      expect(pill?.textContent?.trim()).toBe(String(cards.length));
    }
    expect(list.querySelector('a[href="/tasks/m-1"]')).toBeNull();
    expect(list.querySelector('a[href="/tasks/t-released"]')).toBeNull();
    expect(
      screen.getByRole("link", { name: /テーマ切替/ }).getAttribute("href"),
    ).toBe("/tasks/t-ready-1");
  });

  it("sends nothing but GETs and holds no primary button, however much is carried", async () => {
    const fetchMock = stubFetch({
      control: () =>
        jsonResponse(
          plane({
            mergeable: [summary("t-a", "approved"), summary("t-b", "approved")],
            pending_merges: [pendingMerge("m-1")],
            pending_reviews: [summary("r-1", "ready", "review")],
            releasable: [{ product_id: PRODUCT, task_count: 1 }],
          }),
        ),
    });

    render(Home);
    await waitFor(() =>
      expect(region("control").dataset.state).toBe("success"),
    );

    // Stranded work is drawn, and there is nothing on the page that would
    // act on it: no button at all, and so no primary one.
    expect(screen.queryAllByRole("button")).toHaveLength(0);
    expect(document.querySelector(".primary")).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(writes(fetchMock)).toHaveLength(0);
  });

  it("leaves the tasks the panel draws out of the status groups", async () => {
    stubFetch({
      control: () =>
        jsonResponse(
          plane({
            pending_reviews: [
              summary("r-1", "ready", "review", "レビュー: t-done"),
            ],
            unreviewed: [summary("t-done", "done")],
            mergeable: [summary("t-approved", "approved")],
          }),
        ),
      tasks: () =>
        jsonResponse([
          ...TASKS,
          summary("r-1", "ready", "review", "レビュー: t-done"),
          summary("t-approved", "approved"),
        ]),
    });

    render(Home);
    await waitFor(() =>
      expect(region("control").dataset.state).toBe("success"),
    );
    await waitFor(() => expect(region("tasks").dataset.state).toBe("success"));

    const list = region("tasks");
    for (const id of ["r-1", "t-done", "t-approved", "m-1"]) {
      expect(list.querySelector(`a[href="/tasks/${id}"]`)).toBeNull();
    }
    // Each of them has exactly one home, and it is on the panel.
    const panel = region("control");
    for (const id of ["r-1", "t-done", "t-approved"]) {
      expect(panel.querySelectorAll(`a[href="/tasks/${id}"]`)).toHaveLength(1);
    }
    // Untouched work still stands in its group.
    expect(list.querySelector('a[href="/tasks/t-ready-2"]')).not.toBeNull();
  });

  it("shows a stopped merge's cause off the control payload, asking nothing else", async () => {
    let jammed = true;
    const fetchMock = stubFetch({
      control: () =>
        jsonResponse(
          plane({
            pending_merges: [
              jammed
                ? pendingMerge(
                    "m-1",
                    "blocked",
                    "rebase conflict:\n  src/task.rs",
                  )
                : pendingMerge("m-1"),
              pendingMerge("m-2"),
              pendingMerge("m-9", "ready", null, "sunny-side/other"),
            ],
          }),
        ),
    });

    render(Home);
    await waitFor(() =>
      expect(region("control").dataset.state).toBe("success"),
    );

    const reason = document.querySelector("[data-reason]");
    expect(reason?.textContent).toContain("src/task.rs");
    expect(region("control").textContent).toContain("他 1 件が待機中");
    // The reason rides along with the queue, so nothing is fetched per card:
    // there is no second request to fail on its own, and none to land out of
    // order over a newer one.
    for (const id of ["m-1", "m-2", "m-9"]) {
      expect(callsTo(fetchMock, `/api/tasks/${id}`)).toHaveLength(0);
    }
    expect(callsTo(fetchMock, "/api/control")).toHaveLength(1);
    expect(writes(fetchMock)).toHaveLength(0);

    // A jam that clears takes its reason with it: the panel holds no cause of
    // its own that a later payload would have to remember to overwrite.
    jammed = false;
    setVisibility("hidden");
    setVisibility("visible");
    await waitFor(() =>
      expect(callsTo(fetchMock, "/api/control")).toHaveLength(2),
    );
    await waitFor(() =>
      expect(document.querySelector("[data-reason]")).toBeNull(),
    );
    expect(region("control").textContent).not.toContain("src/task.rs");
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
              plane({ releasable: [{ product_id: PRODUCT, task_count: 1 }] }),
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
