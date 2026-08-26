import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ApiError,
  fetchControl,
  fetchTasks,
  postRelease,
  postTaskStatus,
  type TaskCard,
} from "./api";
import { setSessionCsrf } from "./auth";

const CARD: TaskCard = {
  id: "alpha",
  title: "見本タスク",
  body: "本文",
  status: "ready",
  kind: "normal",
  product_id: "sunny-side/task-server",
  priority: 0,
  branch: null,
  claimed_by: null,
  claim_id: null,
  claimed_at: null,
  claim_expires_at: null,
  commit_sha: null,
  verification: null,
  release_tag: null,
  created_at: "2026-08-15T10:00:00Z",
  updated_at: "2026-08-15T12:00:00Z",
  available_transitions: ["wip", "blocked"],
};

function jsonResponse(payload: unknown): Response {
  return new Response(JSON.stringify(payload), { status: 200 });
}

describe("api", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("requests the unfiltered task list when no status is given", async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(jsonResponse([]));
    vi.stubGlobal("fetch", fetchMock);

    await fetchTasks();

    expect(String(fetchMock.mock.calls[0][0])).toBe("/api/tasks");
  });

  it("appends the status query when a status is given", async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(jsonResponse([]));
    vi.stubGlobal("fetch", fetchMock);

    await fetchTasks(undefined, "instant:merge");

    expect(String(fetchMock.mock.calls[0][0])).toBe(
      "/api/tasks?status=instant%3Amerge",
    );
  });

  it("posts a status transition with the auth headers and returns the card", async () => {
    setSessionCsrf("csrf-token-1");
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse(CARD));
    vi.stubGlobal("fetch", fetchMock);

    const card = await postTaskStatus("T 1", "ready");

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [input, init] = fetchMock.mock.calls[0];
    expect(String(input)).toBe("/api/tasks/T%201/status");
    expect(init?.method).toBe("POST");
    expect(init?.body).toBe(JSON.stringify({ status: "ready" }));
    const headers = new Headers(init?.headers);
    expect(headers.get("content-type")).toBe("application/json");
    expect(headers.get("X-CSRF-Token")).toBe("csrf-token-1");
    expect(card).toEqual(CARD);
  });

  it("reads the control plane from a plain GET", async () => {
    const plane = {
      mergeable: [],
      pending_merges: [],
      pending_reviews: [],
      unreviewed: [],
      releasable: [{ product_id: "sunny-side/task-server", task_count: 3 }],
    };
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse(plane));
    vi.stubGlobal("fetch", fetchMock);

    const control = await fetchControl();

    const [input, init] = fetchMock.mock.calls[0];
    expect(String(input)).toBe("/api/control");
    expect(init?.method).toBeUndefined();
    expect(control).toEqual(plane);
  });

  it("posts a release with product and tag and returns the released set", async () => {
    setSessionCsrf("csrf-token-3");
    const result = {
      product_id: "sunny-side/task-server",
      tag: "v0.2.0",
      released: [],
    };
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse(result));
    vi.stubGlobal("fetch", fetchMock);

    const released = await postRelease("sunny-side/task-server", "v0.2.0");

    const [input, init] = fetchMock.mock.calls[0];
    expect(String(input)).toBe("/api/releases");
    expect(init?.method).toBe("POST");
    expect(init?.body).toBe(
      JSON.stringify({ product_id: "sunny-side/task-server", tag: "v0.2.0" }),
    );
    const headers = new Headers(init?.headers);
    expect(headers.get("content-type")).toBe("application/json");
    expect(headers.get("X-CSRF-Token")).toBe("csrf-token-3");
    expect(released).toEqual(result);
  });

  it("carries the server's refusal message and code to the caller", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: "product sunny-side/task-server has nothing to release",
            code: "conflict",
          }),
          { status: 409, headers: { "content-type": "application/json" } },
        ),
      ),
    );

    const error = await postRelease("sunny-side/task-server", "v0.2.0").then(
      () => {
        throw new Error("postRelease resolved but the server refused");
      },
      (rejected: unknown) => rejected,
    );

    expect(error).toBeInstanceOf(ApiError);
    const refusal = error as ApiError;
    expect(refusal.status).toBe(409);
    expect(refusal.code).toBe("conflict");
    expect(refusal.message).toBe(
      "product sunny-side/task-server has nothing to release",
    );
  });

  it("rejects when the server refuses the transition", async () => {
    vi.stubGlobal(
      "fetch",
      vi
        .fn<typeof fetch>()
        .mockResolvedValue(new Response("nope", { status: 409 })),
    );

    await expect(postTaskStatus("alpha", "released")).rejects.toThrow(
      "HTTP 409",
    );
  });
});
