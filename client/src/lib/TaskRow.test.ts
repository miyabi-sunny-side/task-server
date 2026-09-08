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
