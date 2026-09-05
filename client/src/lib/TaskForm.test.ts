import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import TaskForm from "./TaskForm.svelte";
afterEach(cleanup);
it("keeps invalid save focusable, submits source text, and preserves it on a refusal", async () => {
  const onsave = vi
    .fn()
    .mockRejectedValueOnce(new Error("保存できません"))
    .mockResolvedValueOnce(undefined);
  const onclose = vi.fn();
  render(TaskForm, { title: "新規タスク", onsave, onclose });
  const save = screen.getByRole("button", { name: "保存" });
  expect(save.getAttribute("aria-disabled")).toBe("true");
  expect((save as HTMLButtonElement).disabled).toBe(false);
  await fireEvent.submit(document.querySelector("form")!);
  expect(onsave).not.toHaveBeenCalled();
  await fireEvent.input(screen.getByLabelText("product"), {
    target: { value: "sunny-side/task-server" },
  });
  await fireEvent.input(screen.getByLabelText("title"), {
    target: { value: "新しい作業" },
  });
  await fireEvent.input(screen.getByLabelText("body"), {
    target: { value: "## 指示\n変更内容" },
  });
  await fireEvent.submit(document.querySelector("form")!);
  await screen.findByRole("alert");
  expect((screen.getByLabelText("body") as HTMLTextAreaElement).value).toBe(
    "## 指示\n変更内容",
  );
  expect(onclose).not.toHaveBeenCalled();
  await fireEvent.submit(document.querySelector("form")!);
  await waitFor(() => expect(onclose).toHaveBeenCalledOnce());
  expect(onsave).toHaveBeenLastCalledWith({
    product_id: "sunny-side/task-server",
    title: "新しい作業",
    body: "## 指示\n変更内容",
  });
});
it("does not replace an edit draft when background data changes", async () => {
  const initial = { product_id: "org/repo", title: "元の題名", body: "本文" };
  const props = {
    title: "タスクを編集",
    initial,
    onsave: vi.fn(),
    onclose: vi.fn(),
  };
  const { rerender } = render(TaskForm, props);
  await fireEvent.input(screen.getByLabelText("title"), {
    target: { value: "入力途中" },
  });
  await rerender({ ...props, initial: { ...initial, title: "server 更新" } });
  expect((screen.getByLabelText("title") as HTMLInputElement).value).toBe(
    "入力途中",
  );
});
