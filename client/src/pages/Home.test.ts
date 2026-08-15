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

const PRODUCT = "sunny-side/task-server";

function summary(
  id: string,
  status: string,
  kind = "normal",
  title = `task ${id}`,
): Summary {
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

const EMPTY_PLANE = { mergeable: [], pending_merges: [], releasable: [] };

function jsonResponse(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { "content-type": "application/json" },
  });
}

interface Scenario {
  control?: () => Response | Promise<Response>;
  tasks?: () => Response | Promise<Response>;
  merge?: (taskId: string) => Response | Promise<Response>;
  release?: (body: { product_id: string; tag: string }) => Response;
}

function stubFetch(scenario: Scenario) {
  const fetchMock = vi.fn<typeof fetch>(async (input, init) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    if (url === "/api/control" && method === "GET") {
      return (scenario.control ?? (() => jsonResponse(EMPTY_PLANE)))();
    }
    if (url.startsWith("/api/tasks") && method === "GET") {
      return (scenario.tasks ?? (() => jsonResponse(TASKS)))();
    }
    if (url === "/api/merges" && method === "POST") {
      const body = JSON.parse(String(init?.body)) as { task_id: string };
      return (scenario.merge ?? ((id: string) => jsonResponse({ id }, 201)))(
        body.task_id,
      );
    }
    if (url === "/api/releases" && method === "POST") {
      const body = JSON.parse(String(init?.body)) as {
        product_id: string;
        tag: string;
      };
      return (
        scenario.release ??
        ((sent: { product_id: string; tag: string }) =>
          jsonResponse({ ...sent, released: [] }))
      )(body);
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

function resultLine(): HTMLElement {
  const found = document.querySelector<HTMLElement>("[data-result]");
  if (!found) {
    throw new Error("result line was not found");
  }
  return found;
}

describe("Home", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("the two regions load and fail independently", async () => {
    // The list request fails; the control panel must still work.
    const listDown = stubFetch({
      control: () =>
        jsonResponse({
          mergeable: [summary("t-done", "done")],
          pending_merges: [],
          releasable: [],
        }),
      tasks: () => Promise.reject(new Error("offline")),
    });

    render(Home);
    expect(region("control").dataset.state).toBe("loading");
    expect(region("tasks").dataset.state).toBe("loading");

    await waitFor(() =>
      expect(region("control").dataset.state).toBe("success"),
    );
    await waitFor(() => expect(region("tasks").dataset.state).toBe("error"));

    const merge = screen.getByRole("button", { name: "merge" });
    expect(merge.getAttribute("aria-disabled")).toBeNull();
    await fireEvent.click(merge);
    await waitFor(() =>
      expect(callsTo(listDown, "/api/merges", "POST")).toHaveLength(1),
    );

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

  it("merge issues one request per candidate and reports partial failure", async () => {
    const mergeable = [
      summary("t-a", "done"),
      summary("t-b", "done"),
      summary("t-c", "done"),
    ];
    let controlCalls = 0;
    const fetchMock = stubFetch({
      control: () => {
        controlCalls += 1;
        return controlCalls === 1
          ? jsonResponse({
              mergeable,
              pending_merges: [],
              releasable: [],
            })
          : jsonResponse({
              mergeable: [summary("t-b", "done")],
              pending_merges: [
                summary("m-a", "ready", "instant:merge"),
                summary("m-c", "ready", "instant:merge"),
              ],
              releasable: [],
            });
      },
      merge: (taskId) =>
        taskId === "t-b"
          ? jsonResponse(
              {
                error: "task t-b already has a merge in flight",
                code: "conflict",
              },
              409,
            )
          : jsonResponse({ id: `m-${taskId}` }, 201),
    });

    render(Home);
    await waitFor(() =>
      expect(region("control").dataset.state).toBe("success"),
    );

    await fireEvent.click(screen.getByRole("button", { name: "merge" }));

    await waitFor(() =>
      expect(callsTo(fetchMock, "/api/merges", "POST")).toHaveLength(3),
    );
    const sent = callsTo(fetchMock, "/api/merges", "POST").map(
      ([, init]) =>
        (JSON.parse(String(init?.body)) as { task_id: string }).task_id,
    );
    expect(sent.sort()).toEqual(["t-a", "t-b", "t-c"]);

    await waitFor(() => {
      const alert = screen.getByRole("alert");
      expect(alert.textContent).toContain("2");
      expect(alert.textContent).toContain("already has a merge in flight");
    });

    // Both regions reload once the batch settles, whatever the outcome.
    await waitFor(() =>
      expect(callsTo(fetchMock, "/api/control")).toHaveLength(2),
    );
    await waitFor(() =>
      expect(callsTo(fetchMock, "/api/tasks")).toHaveLength(2),
    );

    cleanup();
    vi.unstubAllGlobals();

    const idle = stubFetch({ control: () => jsonResponse(EMPTY_PLANE) });
    render(Home);
    await waitFor(() => expect(region("control").dataset.state).toBe("empty"));

    const disabled = screen.getByRole("button", { name: "merge" });
    expect(disabled.getAttribute("aria-disabled")).toBe("true");
    expect(disabled.hasAttribute("disabled")).toBe(false);
    const describedBy = disabled.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    expect(
      document.getElementById(String(describedBy))?.textContent?.trim(),
    ).toBeTruthy();

    await fireEvent.click(disabled);
    expect(callsTo(idle, "/api/merges", "POST")).toHaveLength(0);
  });

  it("release posts product and tag, and keeps the modal open on refusal", async () => {
    let refuse = true;
    const fetchMock = stubFetch({
      control: () =>
        jsonResponse({
          mergeable: [],
          pending_merges: [],
          releasable: [
            { product_id: "sunny-side/one", task_count: 2 },
            { product_id: "sunny-side/two", task_count: 3 },
          ],
        }),
      release: (body) =>
        refuse
          ? jsonResponse(
              {
                error: "product sunny-side/two has nothing to release",
                code: "conflict",
              },
              409,
            )
          : jsonResponse({ ...body, released: [summary("t-x", "released")] }),
    });

    render(Home);
    await waitFor(() =>
      expect(region("control").dataset.state).toBe("success"),
    );

    await fireEvent.click(screen.getByRole("button", { name: "release" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog).toBeTruthy();

    const confirm = screen.getByRole("button", { name: "release する" });
    expect(confirm.getAttribute("aria-disabled")).toBe("true");

    const radios = screen.getAllByRole("radio");
    expect(radios).toHaveLength(2);
    expect(radios[0].getAttribute("aria-checked")).toBe("true");
    await fireEvent.click(radios[1]);

    const tag = screen.getByLabelText("tag");
    await fireEvent.input(tag, { target: { value: "   " } });
    expect(
      screen
        .getByRole("button", { name: "release する" })
        .getAttribute("aria-disabled"),
    ).toBe("true");

    await fireEvent.input(tag, { target: { value: "v0.2.0" } });
    expect(
      screen
        .getByRole("button", { name: "release する" })
        .getAttribute("aria-disabled"),
    ).toBeNull();

    await fireEvent.click(screen.getByRole("button", { name: "release する" }));

    await waitFor(() =>
      expect(callsTo(fetchMock, "/api/releases", "POST")).toHaveLength(1),
    );
    expect(
      JSON.parse(
        String(callsTo(fetchMock, "/api/releases", "POST")[0][1]?.body),
      ),
    ).toEqual({ product_id: "sunny-side/two", tag: "v0.2.0" });

    // Refused: the modal stays open, the tag survives, the reason is inside.
    await waitFor(() => {
      const banner = screen.getByRole("dialog").querySelector('[role="alert"]');
      expect(banner?.textContent).toContain("nothing to release");
    });
    expect(screen.getByLabelText<HTMLInputElement>("tag").value).toBe("v0.2.0");

    refuse = false;
    const controlBefore = callsTo(fetchMock, "/api/control").length;
    const tasksBefore = callsTo(fetchMock, "/api/tasks").length;
    await fireEvent.click(screen.getByRole("button", { name: "release する" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(resultLine().textContent?.trim()).not.toBe("");
    await waitFor(() =>
      expect(callsTo(fetchMock, "/api/control").length).toBe(controlBefore + 1),
    );
    await waitFor(() =>
      expect(callsTo(fetchMock, "/api/tasks").length).toBe(tasksBefore + 1),
    );
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: "release" }),
    );
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
});
