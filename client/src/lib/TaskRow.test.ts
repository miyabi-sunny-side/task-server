import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import { tick } from "svelte";
import TaskRow from "./TaskRow.svelte";

const item = {
  id: "artificial",
  title: "人工タスク",
  status: "draft",
  kind: "normal",
  product_id: "test/only",
  priority: 0,
  updated_at: "2026-09-08",
};
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function pointer(row: HTMLElement, type: string, x = 20) {
  // jsdom has no PointerEvent constructor; retain the real event properties.
  const event = new Event(type, { bubbles: true });
  Object.assign(event, {
    pointerType: "touch",
    isPrimary: true,
    button: 0,
    clientX: x,
    clientY: 20,
  });
  row.dispatchEvent(event);
}

it("opens after a stationary hold, suppresses its click, then permits a fresh tap", async () => {
  vi.useFakeTimers();
  render(TaskRow, { item, onupdated: vi.fn() });
  const row = screen.getByRole("link");
  pointer(row, "pointerdown");
  await vi.advanceTimersByTimeAsync(499);
  expect(screen.queryByRole("menu")).toBeNull();
  await vi.advanceTimersByTimeAsync(1);
  await tick();
  expect(screen.getByRole("menu")).toBeTruthy();
  pointer(row, "pointerup");
  expect(
    await fireEvent.mouseDown(
      screen.getByRole("button", { name: "タスクメニューを閉じる" }),
    ),
  ).toBe(false);
  expect(
    await fireEvent.click(
      screen.getByRole("button", { name: "タスクメニューを閉じる" }),
    ),
  ).toBe(false);
  expect(screen.getByRole("menu")).toBeTruthy();
  await fireEvent.keyDown(window, { key: "Escape" });
  expect(document.activeElement).toBe(row);
  pointer(row, "pointerdown");
  pointer(row, "pointerup");
  const click = new MouseEvent("click", { bubbles: true, cancelable: true });
  // Observe whether the row prevents normal activation without navigating jsdom.
  row.addEventListener("click", (e) => e.stopPropagation(), { once: true });
  row.dispatchEvent(click);
  expect(click.defaultPrevented).toBe(false);
});

it.each([
  "pointermove",
  "pointercancel",
  "pointerleave",
  "pointerup",
  "scroll",
])("cancels a hold on %s", async (type) => {
  vi.useFakeTimers();
  render(TaskRow, { item });
  const row = screen.getByRole("link");
  pointer(row, "pointerdown");
  if (type === "scroll") document.dispatchEvent(new Event("scroll"));
  else pointer(row, type, 40);
  await vi.advanceTimersByTimeAsync(600);
  expect(screen.queryByRole("menu")).toBeNull();
});

it("ignores an already queued scroll but closes when its row actually moves", async () => {
  render(TaskRow, { item });
  const row = screen.getByRole("link");
  let top = 20;
  vi.spyOn(row, "getBoundingClientRect").mockImplementation(
    () => ({ top, bottom: top + 80, left: 12 }) as DOMRect,
  );
  await fireEvent.contextMenu(row);
  await fireEvent.scroll(document);
  expect(screen.getByRole("menu")).toBeTruthy();
  top = 0;
  await fireEvent.scroll(document);
  expect(screen.queryByRole("menu")).toBeNull();
  expect(document.activeElement).toBe(row);
});

it.each(["blocked", "cancelled"])(
  "confirms %s, cancels without a write and submits only once",
  async (status) => {
    let finish!: (response: Response) => void;
    const fetchMock = vi.fn(
      (_url: string, _init?: RequestInit) =>
        new Promise<Response>((resolve) => {
          finish = resolve;
        }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const onupdated = vi.fn();
    render(TaskRow, { item, onupdated });
    const row = screen.getByRole("link");
    const label = status === "blocked" ? "Blockする" : "Cancelする";
    await fireEvent.contextMenu(row);
    expect(screen.queryByRole("menuitem", { name: "詳細を開く" })).toBeNull();
    await fireEvent.click(screen.getByRole("menuitem", { name: label }));
    expect(screen.getByRole("dialog").textContent).toContain(item.title);
    expect(fetchMock).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole("button", { name: "取りやめ" }));
    expect(document.activeElement).toBe(row);
    await fireEvent.contextMenu(row);
    await fireEvent.click(screen.getByRole("menuitem", { name: label }));
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();
    await fireEvent.contextMenu(row);
    await fireEvent.click(screen.getByRole("menuitem", { name: label }));
    const confirm = screen.getByRole("button", { name: label });
    await fireEvent.click(confirm);
    await fireEvent.click(confirm);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0]).toEqual([
      "/api/tasks/artificial/status",
      expect.objectContaining({ body: JSON.stringify({ status }) }),
    ]);
    finish(new Response(JSON.stringify({ ...item, status }), { status: 200 }));
    await vi.waitFor(() =>
      expect(onupdated).toHaveBeenCalledWith(
        expect.objectContaining({ status }),
      ),
    );
    vi.unstubAllGlobals();
  },
);

it("keeps a refused confirmation open for retry without updating the row", async () => {
  const fetchMock = vi.fn().mockResolvedValue(
    new Response(JSON.stringify({ error: "transition refused" }), {
      status: 409,
    }),
  );
  vi.stubGlobal("fetch", fetchMock);
  const onupdated = vi.fn();
  render(TaskRow, { item, onupdated });
  await fireEvent.contextMenu(screen.getByRole("link"));
  await fireEvent.click(screen.getByRole("menuitem", { name: "Cancelする" }));
  await fireEvent.click(screen.getByRole("button", { name: "Cancelする" }));
  expect((await screen.findByRole("alert")).textContent).toBe(
    "transition refused",
  );
  expect(screen.getByRole("dialog")).toBeTruthy();
  expect(onupdated).not.toHaveBeenCalled();
  expect(screen.getByRole("link").textContent).toContain("draft");
});

it("copies the encoded detail URL and announces success only after clipboard completion", async () => {
  let finish!: () => void;
  const writeText = vi.fn(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });
  render(TaskRow, { item: { ...item, id: "a #日本" } });
  await fireEvent.contextMenu(screen.getByRole("link"));
  await fireEvent.click(screen.getByRole("menuitem", { name: "URLをコピー" }));
  expect(writeText).toHaveBeenCalledWith(
    `${window.location.origin}/tasks/a%20%23%E6%97%A5%E6%9C%AC`,
  );
  expect(screen.queryByRole("status")).toBeNull();
  finish();
  expect((await screen.findByRole("status")).textContent).toBe(
    "URLをコピーしました",
  );
  writeText.mockRejectedValueOnce(new Error("denied"));
  await fireEvent.click(screen.getByRole("menuitem", { name: "URLをコピー" }));
  expect((await screen.findByRole("alert")).textContent).toBe(
    "URLのコピーに失敗しました",
  );
  expect(screen.queryByRole("status")).toBeNull();
});

it.each(["draft", "ready", "wip", "blocked", "cancelled"])(
  "omits same-state actions for %s and all mutations when archived",
  async (status) => {
    const { rerender } = render(TaskRow, {
      item: { ...item, status },
      onupdated: vi.fn(),
    });
    await fireEvent.contextMenu(screen.getByRole("link"));
    expect(!!screen.queryByRole("menuitem", { name: "Blockする" })).toBe(
      status !== "blocked",
    );
    expect(!!screen.queryByRole("menuitem", { name: "Cancelする" })).toBe(
      status !== "cancelled",
    );
    await rerender({ item: { ...item, status, archived: true } });
    expect(screen.getAllByRole("menuitem").map((el) => el.textContent)).toEqual(
      ["URLをコピー"],
    );
  },
);
