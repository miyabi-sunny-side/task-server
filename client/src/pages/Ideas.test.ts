import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import Ideas from "./Ideas.svelte";

const IDEA = {
  id: "i1",
  title: "棚の在庫をNFCで数える",
  product_id: "org/repo",
  created_at: "2026-09-20T00:00:00Z",
  updated_at: "2026-09-25T10:00:00Z",
  revision: 3,
  archived: false,
  archived_at: null,
  task_id: "idea-i1",
  promoted_at: "2026-09-25T10:00:00Z",
};

const json = (value: unknown, status = 200) =>
  Promise.resolve(new Response(JSON.stringify(value), { status }));

describe("Ideas", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("lists ideas as links with their update time and task marker", async () => {
    const fetchMock = vi.fn<typeof fetch>(() =>
      json([
        IDEA,
        { ...IDEA, id: "i2", title: "二つ目", product_id: null, task_id: null },
      ]),
    );
    vi.stubGlobal("fetch", fetchMock);

    render(Ideas, { props: { archived: false } });

    const link = await screen.findByRole("link", { name: /棚の在庫/ });
    expect(link.getAttribute("href")).toBe("/ideas/i1");
    expect(link.textContent).toContain("2026-09-25T10:00:00Z");
    expect(link.textContent).toContain("org/repo");
    expect(link.textContent).toContain("タスク化済み");
    const second = screen.getByRole("link", { name: /二つ目/ });
    expect(second.textContent).not.toContain("タスク化済み");
    expect(String(fetchMock.mock.calls[0][0])).toBe("/api/ideas");
    expect(
      screen.getByRole("link", { name: "アーカイブ" }).getAttribute("href"),
    ).toBe("/ideas/archived");
  });

  it("adds an idea from its title alone and keeps the field ready for the next", async () => {
    let listed: unknown[] = [];
    const fetchMock = vi.fn<typeof fetch>((input, init) => {
      if (init?.method === "POST") {
        listed = [IDEA];
        return json(IDEA, 201);
      }
      return json(listed);
    });
    vi.stubGlobal("fetch", fetchMock);
    render(Ideas, { props: { archived: false } });
    await screen.findByText("アイデアがありません");

    const input = screen.getByLabelText("新しいアイデア");
    await fireEvent.input(input, { target: { value: "  棚の在庫  " } });
    await fireEvent.submit(input.closest("form")!);

    await screen.findByRole("link", { name: /棚の在庫/ });
    const post = fetchMock.mock.calls.find(
      ([, init]) => init?.method === "POST",
    );
    expect(String(post?.[0])).toBe("/api/ideas");
    expect(post?.[1]?.body).toBe(JSON.stringify({ title: "棚の在庫" }));
    expect((input as HTMLInputElement).value).toBe("");
  });

  it("keeps the typed title and says why when adding fails", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>((_input, init) =>
        init?.method === "POST"
          ? json({ error: "書き込めません", code: "io" }, 500)
          : json([]),
      ),
    );
    render(Ideas, { props: { archived: false } });
    await screen.findByText("アイデアがありません");

    const input = screen.getByLabelText("新しいアイデア") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "消えない" } });
    await fireEvent.submit(input.closest("form")!);

    expect((await screen.findByRole("alert")).textContent).toContain(
      "書き込めません",
    );
    expect(input.value).toBe("消えない");
  });

  it("a blank title does not post", async () => {
    const fetchMock = vi.fn<typeof fetch>(() => json([]));
    vi.stubGlobal("fetch", fetchMock);
    render(Ideas, { props: { archived: false } });
    await screen.findByText("アイデアがありません");

    const input = screen.getByLabelText("新しいアイデア");
    await fireEvent.submit(input.closest("form")!);

    expect(
      fetchMock.mock.calls.some(([, init]) => init?.method === "POST"),
    ).toBe(false);
  });

  it("the archive lists archived ideas without the add form", async () => {
    const fetchMock = vi.fn<typeof fetch>(() =>
      json([{ ...IDEA, archived: true, archived_at: "2026-09-26T00:00:00Z" }]),
    );
    vi.stubGlobal("fetch", fetchMock);

    render(Ideas, { props: { archived: true } });

    const link = await screen.findByRole("link", { name: /棚の在庫/ });
    expect(link.textContent).toContain("2026-09-26T00:00:00Z");
    expect(String(fetchMock.mock.calls[0][0])).toBe("/api/ideas?archived=true");
    expect(screen.queryByLabelText("新しいアイデア")).toBeNull();
    expect(
      screen.getByRole("link", { name: "アイデア一覧" }).getAttribute("href"),
    ).toBe("/ideas");
  });

  it("opens the row menu from the keyboard with archive as its only item", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>(() => json([IDEA])),
    );
    render(Ideas, { props: { archived: false } });
    const row = await screen.findByRole("link", { name: /棚の在庫/ });
    expect(row.getAttribute("aria-haspopup")).toBe("menu");

    await fireEvent.keyDown(row, { key: "F10", shiftKey: true });
    expect(screen.getByRole("menu", { name: IDEA.title })).toBeTruthy();
    expect(screen.getAllByRole("menuitem").map((el) => el.textContent)).toEqual(
      ["アーカイブ"],
    );
    expect(document.activeElement?.textContent).toBe("アーカイブ");
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(row);

    await fireEvent.keyDown(row, { key: "ContextMenu" });
    expect(screen.getByRole("menu")).toBeTruthy();
  });

  it("archives a row after one confirmation, then drops it and focuses the archive link", async () => {
    let archived = false;
    let finish!: () => void;
    const fetchMock = vi.fn<typeof fetch>((_input, init) => {
      if (init?.method === "POST") {
        return new Promise((resolve) => {
          finish = () => {
            archived = true;
            resolve(
              new Response(
                JSON.stringify({
                  ...IDEA,
                  archived: true,
                  archived_at: "2026-09-26T00:00:00Z",
                }),
              ),
            );
          };
        });
      }
      return json(archived ? [] : [IDEA]);
    });
    vi.stubGlobal("fetch", fetchMock);
    const posts = () =>
      fetchMock.mock.calls.filter(([, init]) => init?.method === "POST");
    render(Ideas, { props: { archived: false } });
    const row = await screen.findByRole("link", { name: /棚の在庫/ });

    await fireEvent.contextMenu(row);
    await fireEvent.click(screen.getByRole("menuitem", { name: "アーカイブ" }));
    const dialog = screen.getByRole("dialog", { name: "アイデアをアーカイブ" });
    expect(dialog.textContent).toContain(IDEA.title);
    await fireEvent.click(screen.getByRole("button", { name: "取りやめ" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(row);

    await fireEvent.contextMenu(row);
    await fireEvent.click(screen.getByRole("menuitem", { name: "アーカイブ" }));
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(posts()).toHaveLength(0);

    await fireEvent.contextMenu(row);
    await fireEvent.click(screen.getByRole("menuitem", { name: "アーカイブ" }));
    const confirm = screen
      .getByRole("dialog")
      .querySelector<HTMLButtonElement>("button.primary")!;
    await fireEvent.click(confirm);
    await fireEvent.click(confirm);
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(posts()).toHaveLength(1);
    expect(String(posts()[0][0])).toBe("/api/ideas/i1/archive");

    finish();
    await waitFor(() =>
      expect(screen.queryByRole("link", { name: /棚の在庫/ })).toBeNull(),
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    await screen.findByText("アイデアがありません");
    await waitFor(() =>
      expect(document.activeElement?.getAttribute("href")).toBe(
        "/ideas/archived",
      ),
    );
  });

  it("keeps a failed archive in the dialog for retry and leaves the row", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>((_input, init) =>
        init?.method === "POST"
          ? json({ error: "書き込めません", code: "io" }, 500)
          : json([IDEA]),
      ),
    );
    render(Ideas, { props: { archived: false } });
    const row = await screen.findByRole("link", { name: /棚の在庫/ });

    await fireEvent.contextMenu(row);
    await fireEvent.click(screen.getByRole("menuitem", { name: "アーカイブ" }));
    await fireEvent.click(
      screen
        .getByRole("dialog")
        .querySelector<HTMLButtonElement>("button.primary")!,
    );

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("書き込めません");
    expect(screen.getByRole("dialog").contains(alert)).toBe(true);
    expect(screen.getByRole("link", { name: /棚の在庫/ })).toBe(row);
  });

  it("archived rows are plain links without a menu", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>(() =>
        json([
          { ...IDEA, archived: true, archived_at: "2026-09-26T00:00:00Z" },
        ]),
      ),
    );
    render(Ideas, { props: { archived: true } });
    const row = await screen.findByRole("link", { name: /棚の在庫/ });

    expect(row.hasAttribute("aria-haspopup")).toBe(false);
    await fireEvent.contextMenu(row);
    await fireEvent.keyDown(row, { key: "ContextMenu" });
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("offers a retry when the first load fails", async () => {
    let fail = true;
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>(() => (fail ? json({}, 500) : json([IDEA]))),
    );
    render(Ideas, { props: { archived: false } });

    const retry = await screen.findByRole("button", { name: "再試行" });
    fail = false;
    await fireEvent.click(retry);
    await screen.findByRole("link", { name: /棚の在庫/ });
  });
});
