import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import IdeaDetail from "./IdeaDetail.svelte";
import { router } from "../lib/router.svelte";

const IDEA = {
  id: "i1",
  title: "棚の在庫をNFCで数える",
  body: "調査メモ\n<b>太字ではない</b>",
  product_id: "org/repo",
  created_at: "2026-09-20T00:00:00Z",
  updated_at: "2026-09-25T10:00:00Z",
  revision: 3,
  archived: false,
  archived_at: null,
  task_id: null,
  promoted_at: null,
};

const json = (value: unknown, status = 200) =>
  Promise.resolve(new Response(JSON.stringify(value), { status }));

type Handler = (url: string, init?: RequestInit) => Promise<Response>;

function serve(handler: Handler) {
  const fetchMock = vi.fn<typeof fetch>((input, init) =>
    handler(String(input), init),
  );
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

async function openEditor() {
  await fireEvent.click(await screen.findByRole("button", { name: "編集" }));
  return screen.getByLabelText("body") as HTMLTextAreaElement;
}

describe("IdeaDetail", () => {
  beforeEach(() => window.history.replaceState(null, "", "/ideas/i1"));
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("shows the body as plain text with its metadata", async () => {
    serve(() => json(IDEA));
    render(IdeaDetail, { props: { id: "i1" } });

    expect(
      await screen.findByRole("heading", { name: IDEA.title }),
    ).toBeTruthy();
    const body = screen.getByText(/調査メモ/);
    expect(body.textContent).toContain("<b>太字ではない</b>");
    expect(body.querySelector("b")).toBeNull();
    expect(screen.getByText("org/repo")).toBeTruthy();
  });

  it("saves an edit against the revision it started from", async () => {
    const fetchMock = serve((url, init) =>
      init?.method === "PATCH"
        ? json({ ...IDEA, body: "追記済み", revision: 4 })
        : json(IDEA),
    );
    render(IdeaDetail, { props: { id: "i1" } });

    const body = await openEditor();
    expect(body.value).toBe(IDEA.body);
    await fireEvent.input(body, { target: { value: "追記済み" } });
    await fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await screen.findByText("追記済み");
    const patch = fetchMock.mock.calls.find(
      ([, init]) => init?.method === "PATCH",
    );
    expect(String(patch?.[0])).toBe("/api/ideas/i1");
    expect(JSON.parse(String(patch?.[1]?.body))).toEqual({
      title: IDEA.title,
      product_id: "org/repo",
      body: "追記済み",
      expected_revision: 3,
    });
    expect(screen.queryByLabelText("本文")).toBeNull();
  });

  it("a conflict keeps the draft, shows the latest and can overwrite from it", async () => {
    const latest = { ...IDEA, body: "他の人の追記", revision: 5 };
    let saved = false;
    let loads = 0;
    serve((_url, init) => {
      if (init?.method === "PATCH") {
        const sent = JSON.parse(String(init.body));
        if (sent.expected_revision !== 5) {
          return json({ error: "revision mismatch", code: "conflict" }, 409);
        }
        saved = true;
        return json({ ...latest, body: sent.body, revision: 6 });
      }
      // The page loaded revision 3, then someone else saved revision 5.
      loads += 1;
      if (loads === 1) return json(IDEA);
      return json(
        saved ? { ...latest, body: "私の下書き", revision: 6 } : latest,
      );
    });
    render(IdeaDetail, { props: { id: "i1" } });

    const body = await openEditor();
    await fireEvent.input(body, { target: { value: "私の下書き" } });
    await fireEvent.click(screen.getByRole("button", { name: "保存" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("他の更新");
    expect(await screen.findByText("他の人の追記")).toBeTruthy();
    expect((screen.getByLabelText("body") as HTMLTextAreaElement).value).toBe(
      "私の下書き",
    );

    await fireEvent.click(
      screen.getByRole("button", { name: "この内容で上書き保存" }),
    );
    await waitFor(() => expect(saved).toBe(true));
    await waitFor(() => expect(screen.queryByLabelText("本文")).toBeNull());
    expect(screen.getByText("私の下書き")).toBeTruthy();
  });

  it("a conflict can be resolved by discarding the draft for the latest", async () => {
    const latest = { ...IDEA, body: "他の人の追記", revision: 5 };
    let first = true;
    serve((_url, init) => {
      if (init?.method === "PATCH") {
        return json({ error: "revision mismatch", code: "conflict" }, 409);
      }
      if (first) {
        first = false;
        return json(IDEA);
      }
      return json(latest);
    });
    render(IdeaDetail, { props: { id: "i1" } });

    const body = await openEditor();
    await fireEvent.input(body, { target: { value: "捨てる下書き" } });
    await fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await fireEvent.click(
      await screen.findByRole("button", { name: "下書きを破棄" }),
    );

    expect(screen.queryByLabelText("本文")).toBeNull();
    expect(screen.getByText("他の人の追記")).toBeTruthy();
  });

  it("a failed save keeps the draft and can be retried", async () => {
    let fail = true;
    serve((_url, init) => {
      if (init?.method === "PATCH") {
        return fail
          ? json({ error: "書き込めません", code: "io" }, 500)
          : json({ ...IDEA, body: "残る", revision: 4 });
      }
      return json(IDEA);
    });
    render(IdeaDetail, { props: { id: "i1" } });

    const body = await openEditor();
    await fireEvent.input(body, { target: { value: "残る" } });
    await fireEvent.click(screen.getByRole("button", { name: "保存" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "書き込めません",
    );
    expect(body.value).toBe("残る");

    fail = false;
    await fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(screen.queryByLabelText("本文")).toBeNull());
  });

  it("a background reload neither resets the draft nor moves focus", async () => {
    vi.useFakeTimers();
    const fetchMock = serve(() => json(IDEA));
    render(IdeaDetail, { props: { id: "i1" } });
    await vi.waitFor(() => screen.getByRole("button", { name: "編集" }));
    await fireEvent.click(screen.getByRole("button", { name: "編集" }));
    const body = screen.getByLabelText("body") as HTMLTextAreaElement;
    await fireEvent.input(body, { target: { value: "書きかけ" } });
    body.focus();
    const calls = fetchMock.mock.calls.length;

    fetchMock.mockImplementation(() =>
      json({ ...IDEA, body: "裏で更新", revision: 4 }),
    );
    await vi.advanceTimersByTimeAsync(30_000);

    expect(fetchMock.mock.calls.length).toBeGreaterThan(calls);
    expect(body.value).toBe("書きかけ");
    expect(document.activeElement).toBe(body);
  });

  it("archives after confirmation and then offers no edits", async () => {
    let archived = false;
    const fetchMock = serve((url, init) => {
      if (init?.method === "POST" && url.endsWith("/archive")) {
        archived = true;
      }
      return json(
        archived
          ? { ...IDEA, archived: true, archived_at: "2026-09-26T00:00:00Z" }
          : IDEA,
      );
    });
    render(IdeaDetail, { props: { id: "i1" } });

    await fireEvent.click(
      await screen.findByRole("button", { name: "アーカイブ" }),
    );
    const dialog = screen.getByRole("dialog", { name: "アイデアをアーカイブ" });
    await fireEvent.click(
      dialog.querySelector<HTMLButtonElement>("button.primary")!,
    );

    await screen.findByText("アーカイブ済み");
    expect(
      fetchMock.mock.calls.some(
        ([url]) => String(url) === "/api/ideas/i1/archive",
      ),
    ).toBe(true);
    expect(screen.queryByRole("button", { name: "編集" })).toBeNull();
    expect(screen.queryByRole("button", { name: "タスク化" })).toBeNull();
  });

  it("promotes to a draft task and moves to it", async () => {
    const fetchMock = serve((url, init) => {
      if (url === "/api/execution-targets") {
        return json({ labels: ["sandbox", "homeserver"], default: "sandbox" });
      }
      if (init?.method === "POST" && url.endsWith("/promote")) {
        return json({
          idea: { ...IDEA, task_id: "idea-i1", revision: 4 },
          task: { id: "idea-i1", title: IDEA.title },
        });
      }
      return json(IDEA);
    });
    render(IdeaDetail, { props: { id: "i1" } });

    await fireEvent.click(
      await screen.findByRole("button", { name: "タスク化" }),
    );
    const dialog = screen.getByRole("dialog", { name: "アイデアをタスク化" });
    await waitFor(() =>
      expect((dialog.querySelector("select") as HTMLSelectElement).value).toBe(
        "sandbox",
      ),
    );
    expect(
      (dialog.querySelector("textarea") as HTMLTextAreaElement).value,
    ).toBe(IDEA.body);
    await fireEvent.click(screen.getByRole("button", { name: "タスクを作成" }));

    await waitFor(() =>
      expect(window.location.pathname).toBe("/tasks/idea-i1"),
    );
    expect(router.index).toBe(1);
    const post = fetchMock.mock.calls.find(([url]) =>
      String(url).endsWith("/promote"),
    );
    expect(JSON.parse(String(post?.[1]?.body))).toEqual({
      product_id: "org/repo",
      title: IDEA.title,
      body: IDEA.body,
      execution_target: "sandbox",
    });
  });

  it("a promoted idea links to its task instead of offering promotion", async () => {
    serve(() => json({ ...IDEA, task_id: "idea-i1" }));
    render(IdeaDetail, { props: { id: "i1" } });

    const link = await screen.findByRole("link", { name: "タスクを開く" });
    expect(link.getAttribute("href")).toBe("/tasks/idea-i1");
    expect(screen.queryByRole("button", { name: "タスク化" })).toBeNull();
  });
});
