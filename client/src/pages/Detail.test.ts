import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import Detail from "./Detail.svelte";

const CARD = {
  id: "alpha",
  title: "見本タスク",
  body: "詳細本文",
  status: "draft",
  kind: "normal",
  product_id: "sunny-side/task-server",
  priority: 0,
  branch: null,
  claimed_by: null,
  claim_id: null,
  claimed_at: null,
  claim_expires_at: null,
  commit_sha: "abc1234",
  verification: "cargo test",
  release_tag: null,
  created_at: "2026-08-15T10:00:00Z",
  updated_at: "2026-08-15T12:00:00Z",
  available_transitions: ["ready", "cancelled"],
};

const READY_CARD = {
  ...CARD,
  status: "ready",
  available_transitions: ["wip", "blocked"],
};

describe("Detail", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("loads a task card and posts a status transition then reloads", async () => {
    let current = CARD;
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockImplementation((input, init) => {
        if (String(input).endsWith("/status") && init?.method === "POST") {
          current = READY_CARD;
          return Promise.resolve(
            new Response(JSON.stringify(READY_CARD), { status: 200 }),
          );
        }
        return Promise.resolve(
          new Response(JSON.stringify(current), { status: 200 }),
        );
      });
    vi.stubGlobal("fetch", fetchMock);

    render(Detail, { props: { id: "alpha" } });

    await waitFor(() =>
      expect(screen.getByRole("heading", { name: CARD.title })).toBeTruthy(),
    );
    expect(screen.getByText(CARD.body)).toBeTruthy();
    expect(screen.getByText(CARD.verification)).toBeTruthy();
    expect(screen.getByText(CARD.commit_sha)).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "ready" }));

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "wip" })).toBeTruthy(),
    );
    const post = fetchMock.mock.calls.find(
      ([, init]) => init?.method === "POST",
    );
    expect(post).toBeDefined();
    expect(String(post?.[0])).toBe("/api/tasks/alpha/status");
    expect(post?.[1]?.body).toBe(JSON.stringify({ status: "ready" }));
    expect(screen.queryByRole("button", { name: "cancelled" })).toBe(null);
  });

  it("shows an error message when the transition is refused", async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockImplementation((input, init) => {
        if (String(input).endsWith("/status") && init?.method === "POST") {
          return Promise.resolve(new Response("nope", { status: 409 }));
        }
        return Promise.resolve(
          new Response(JSON.stringify(CARD), { status: 200 }),
        );
      });
    vi.stubGlobal("fetch", fetchMock);

    render(Detail, { props: { id: "alpha" } });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "ready" })).toBeTruthy(),
    );

    await fireEvent.click(screen.getByRole("button", { name: "ready" }));

    await waitFor(() =>
      expect(screen.getByText("操作に失敗しました")).toBeTruthy(),
    );
  });

  it("reloads on the recurring interval while mounted, and stops after unmount", async () => {
    vi.useFakeTimers();
    try {
      const fetchMock = vi
        .fn<typeof fetch>()
        .mockImplementation(() =>
          Promise.resolve(new Response(JSON.stringify(CARD), { status: 200 })),
        );
      vi.stubGlobal("fetch", fetchMock);

      const { unmount } = render(Detail, { props: { id: "alpha" } });
      await vi.waitFor(() =>
        expect(screen.getByRole("heading", { name: CARD.title })).toBeTruthy(),
      );
      expect(fetchMock).toHaveBeenCalledTimes(1);

      await vi.advanceTimersByTimeAsync(30_000);
      expect(fetchMock).toHaveBeenCalledTimes(2);

      unmount();
      await vi.advanceTimersByTimeAsync(60_000);
      expect(fetchMock).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps the drawn card when a background reload fails", async () => {
    let fail = false;
    const fetchMock = vi.fn<typeof fetch>().mockImplementation(() => {
      if (fail) {
        return Promise.reject(new Error("offline"));
      }
      return Promise.resolve(
        new Response(JSON.stringify(CARD), { status: 200 }),
      );
    });
    vi.stubGlobal("fetch", fetchMock);

    render(Detail, { props: { id: "alpha" } });
    await waitFor(() =>
      expect(screen.getByRole("heading", { name: CARD.title })).toBeTruthy(),
    );

    fail = true;
    document.dispatchEvent(new Event("visibilitychange"));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    // Let the rejection settle before asserting the card is still drawn.
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(screen.getByRole("heading", { name: CARD.title })).toBeTruthy();
    expect(screen.getByText(CARD.body)).toBeTruthy();
    expect(screen.queryByText("読み込みに失敗しました")).toBe(null);
  });
});
