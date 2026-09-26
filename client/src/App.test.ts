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

  it("creates from the home header, preserves failed input and returns focus after closing", async () => {
    let tasks: (typeof TASK)[] = [];
    let failSave = true;
    const fetchMock = vi.fn<typeof fetch>(async (input, init) => {
      const url = String(input);
      if (url === "/api/execution-targets")
        return new Response(
          JSON.stringify({
            labels: ["forge", "field", "研究 / 試行"],
            default: "forge",
          }),
        );
      if (url === "/api/tasks" && init?.method === "POST") {
        if (failSave)
          return new Response(JSON.stringify({ error: "save refused" }), {
            status: 409,
          });
        tasks = [{ ...TASK, ...JSON.parse(String(init.body)) }];
        return new Response(JSON.stringify(tasks[0]));
      }
      return new Response(
        JSON.stringify(url === "/api/control" ? { stuck: [] } : tasks),
      );
    });
    vi.stubGlobal("fetch", fetchMock);
    render(App);
    const opener = screen.getByRole("button", { name: "新規タスク" });
    expect(screen.getByRole("banner").contains(opener)).toBe(true);
    expect(screen.getAllByRole("button", { name: "新規タスク" })).toHaveLength(
      1,
    );
    opener.focus();
    await fireEvent.click(opener);
    await fireEvent.input(screen.getByLabelText("product"), {
      target: { value: TASK.product_id },
    });
    await fireEvent.input(screen.getByLabelText("title"), {
      target: { value: TASK.title },
    });
    await fireEvent.click(screen.getByRole("button", { name: "保存" }));
    expect((await screen.findByRole("alert")).textContent).toBe("save refused");
    expect(screen.getByLabelText("title")).toHaveProperty("value", TASK.title);
    failSave = false;
    await fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await screen.findByRole("link", { name: new RegExp(TASK.title) });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(opener);
    await fireEvent.click(opener);
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(opener);
    await fireEvent.click(screen.getByRole("link", { name: "closed" }));
    expect(screen.queryByRole("button", { name: "新規タスク" })).toBeNull();
    await fireEvent.click(screen.getByRole("link", { name: "Task Server" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: "新規タスク" })).toBeTruthy();
  });

  it("restores the products URL and reaches it from the menu", async () => {
    vi.stubGlobal(
      "fetch",
      vi
        .fn<typeof fetch>()
        .mockImplementation(
          async (input) =>
            new Response(
              JSON.stringify(
                String(input) === "/api/execution-targets"
                  ? { labels: [], default: null }
                  : [],
              ),
            ),
        ),
    );
    window.history.replaceState(null, "", "/products");
    render(App);
    await screen.findByText("登録済みのプロダクトがありません");
    await fireEvent.click(screen.getByRole("link", { name: "Task Server" }));
    expect(window.location.pathname).toBe("/");
    await fireEvent.click(screen.getByRole("button", { name: "メニュー" }));
    await fireEvent.click(screen.getByRole("link", { name: "プロダクト一覧" }));
    expect(window.location.pathname).toBe("/products");
    expect(screen.queryByRole("navigation")).toBeNull();
    await screen.findByText("登録済みのプロダクトがありません");
    await fireEvent.click(screen.getByRole("button", { name: "メニュー" }));
    expect(
      screen
        .getByRole("link", { name: "プロダクト一覧" })
        .getAttribute("aria-current"),
    ).toBe("page");
  });

  it("keeps the invariant header and restores a deep detail URL", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>().mockImplementation((input) => {
        if (String(input) === "/api/execution-targets")
          return Promise.resolve(
            new Response(JSON.stringify({ labels: [], default: null })),
          );
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
    expect(header.querySelector('a[href="/ideas"]')).toBeTruthy();
    expect(header.querySelector('a[href="/closed"]')).toBeTruthy();
    expect(screen.getByRole("button", { name: "メニュー" })).toBeTruthy();
    expect(header.querySelectorAll("a, button")).toHaveLength(4);

    await waitFor(() =>
      expect(screen.getByRole("heading", { name: TASK.title })).toBeTruthy(),
    );
    const subHeader = document.querySelector(".sub-header");
    expect(subHeader?.querySelectorAll("a, button")).toHaveLength(0);

    await fireEvent.click(title as HTMLElement);
    expect(window.location.pathname).toBe("/");
  });

  it("navigates to /closed from the header and marks it current", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>().mockImplementation((input) => {
        const url = String(input);
        if (url === "/api/execution-targets")
          return Promise.resolve(
            new Response(JSON.stringify({ labels: [], default: null })),
          );
        const payload =
          url === "/api/tasks" || url === "/api/closed"
            ? []
            : url === "/api/control"
              ? {
                  mergeable: [],
                  pending_merges: [],
                  pending_reviews: [],
                  unreviewed: [],
                  releasable: [],
                }
              : TASK;
        return Promise.resolve(
          new Response(JSON.stringify(payload), { status: 200 }),
        );
      }),
    );

    render(App);

    const header = screen.getByRole("banner");
    const doneLink = header.querySelector('a[href="/closed"]') as HTMLElement;
    expect(doneLink.getAttribute("aria-current")).toBeNull();

    await fireEvent.click(doneLink);

    expect(window.location.pathname).toBe("/closed");
    expect(doneLink.getAttribute("aria-current")).toBe("page");
    await waitFor(() =>
      expect(screen.getByText("閉じたタスクがありません")).toBeTruthy(),
    );
  });
});
