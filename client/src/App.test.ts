import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "./App.svelte";

const TASK = {
  id: "sumi",
  title: "Sumi ダークテーマ",
  body: "本文テキスト",
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
  verification: "ok",
  release_tag: null,
  created_at: "2026-08-15T10:00:00Z",
  updated_at: "2026-08-15T12:00:00Z",
  available_transitions: ["ready"],
};

describe("App", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    window.history.replaceState(null, "", "/");
  });

  it("keeps the invariant header and restores a deep detail URL", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>().mockImplementation((input) => {
        const payload = String(input) === "/api/tasks" ? [TASK] : TASK;
        return Promise.resolve(
          new Response(JSON.stringify(payload), { status: 200 }),
        );
      }),
    );
    window.history.replaceState(null, "", "/tasks/sumi");

    render(App);

    const header = screen.getByRole("banner");
    const title = header.querySelector('a[href="/"]');
    expect(title?.textContent).toContain("Task Server");
    expect(header.querySelector('a[href="/done"]')).toBeTruthy();
    expect(screen.getByRole("button", { name: "メニュー" })).toBeTruthy();
    expect(header.querySelectorAll("a, button")).toHaveLength(3);

    await waitFor(() =>
      expect(screen.getByRole("heading", { name: TASK.title })).toBeTruthy(),
    );
    const subHeader = document.querySelector(".sub-header");
    expect(subHeader?.querySelectorAll("a, button")).toHaveLength(0);

    await fireEvent.click(title as HTMLElement);
    expect(window.location.pathname).toBe("/");
  });

  it("navigates to /done from the header and marks it current", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>().mockImplementation((input) => {
        const url = String(input);
        const payload =
          url === "/api/tasks" || url === "/api/done"
            ? []
            : url === "/api/control"
              ? {
                  mergeable: [],
                  pending_merges: [],
                  pending_reviews: [],
                  unreviewed: [],
                  releasable: [],
                }
              : url === "/api/session"
                ? { user: "test", csrf_token: "test-csrf" }
                : TASK;
        return Promise.resolve(
          new Response(JSON.stringify(payload), { status: 200 }),
        );
      }),
    );

    render(App);

    const header = screen.getByRole("banner");
    const doneLink = header.querySelector('a[href="/done"]') as HTMLElement;
    expect(doneLink.getAttribute("aria-current")).toBeNull();

    await fireEvent.click(doneLink);

    expect(window.location.pathname).toBe("/done");
    expect(doneLink.getAttribute("aria-current")).toBe("page");
    await waitFor(() =>
      expect(screen.getByText("完了したタスクがありません")).toBeTruthy(),
    );
  });
});
