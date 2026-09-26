import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import TaskForm from "./TaskForm.svelte";
beforeEach(() => {
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            labels: ["forge", "field", "研究 / 試行"],
            default: "forge",
          }),
        ),
    ),
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
it("keeps invalid save focusable, submits source text, and preserves it on a refusal", async () => {
  const onsave = vi
    .fn()
    .mockRejectedValueOnce(new Error("保存できません"))
    .mockResolvedValueOnce(undefined);
  const onclose = vi.fn();
  render(TaskForm, { title: "新規タスク", onsave, onclose });
  await screen.findByRole("option", { name: "研究 / 試行" });
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
  await fireEvent.change(screen.getByLabelText("実行先"), {
    target: { value: "研究 / 試行" },
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
    execution_target: "研究 / 試行",
    product_id: "sunny-side/task-server",
    title: "新しい作業",
    body: "## 指示\n変更内容",
  });
});
it("does not replace an edit draft when background data changes", async () => {
  const initial = {
    product_id: "org/repo",
    title: "元の題名",
    body: "本文",
    execution_target: "field",
  };
  const props = {
    title: "タスクを編集",
    initial,
    onsave: vi.fn(),
    onclose: vi.fn(),
  };
  const { rerender } = render(TaskForm, props);
  await screen.findByRole("option", { name: "forge" });
  await fireEvent.input(screen.getByLabelText("title"), {
    target: { value: "入力途中" },
  });
  await fireEvent.change(screen.getByLabelText("実行先"), {
    target: { value: "forge" },
  });
  await rerender({ ...props, initial: { ...initial, title: "server 更新" } });
  expect((screen.getByLabelText("実行先") as HTMLSelectElement).value).toBe(
    "forge",
  );
  expect((screen.getByLabelText("title") as HTMLInputElement).value).toBe(
    "入力途中",
  );
});

it.each([
  { labels: [], default: null },
  { labels: ["forge", "field", "研究 / 試行"], default: null },
])(
  "requires an explicit new destination when there is no external default: %j",
  async (config) => {
    vi.mocked(fetch).mockResolvedValue(new Response(JSON.stringify(config)));
    const onsave = vi.fn();
    render(TaskForm, { title: "新規タスク", onsave, onclose: vi.fn() });
    await waitFor(() =>
      expect(vi.mocked(fetch)).toHaveBeenCalledWith(
        "/api/execution-targets",
        expect.anything(),
      ),
    );
    await fireEvent.input(screen.getByLabelText("product"), {
      target: { value: "org/repo" },
    });
    await fireEvent.input(screen.getByLabelText("title"), {
      target: { value: "new" },
    });
    await fireEvent.submit(document.querySelector("form")!);
    expect(onsave).not.toHaveBeenCalled();
    expect(
      (screen.getByLabelText("実行先") as HTMLSelectElement).disabled,
    ).toBe(config.labels.length === 0);
    if (config.labels.length) {
      await fireEvent.change(screen.getByLabelText("実行先"), {
        target: { value: "field" },
      });
      await fireEvent.submit(document.querySelector("form")!);
      expect(onsave).toHaveBeenCalledWith(
        expect.objectContaining({ execution_target: "field" }),
      );
    } else {
      expect(screen.getByText("実行先が設定されていません")).toBeTruthy();
    }
  },
);

it.each(["retired destination", null])(
  "preserves a historical reference on an unrelated edit: %s",
  async (execution_target) => {
    const onsave = vi.fn();
    render(TaskForm, {
      title: "編集",
      initial: {
        product_id: "org/repo",
        title: "old",
        body: "keep",
        execution_target,
      },
      onsave,
      onclose: vi.fn(),
    });
    await screen.findByRole("option", { name: "forge" });
    if (execution_target)
      expect(
        screen.getByRole("option", {
          name: /retired destination.*現在の設定にありません/,
        }),
      ).toBeTruthy();
    await fireEvent.input(screen.getByLabelText("title"), {
      target: { value: "edited" },
    });
    await fireEvent.submit(document.querySelector("form")!);
    expect(onsave).toHaveBeenCalledWith({
      product_id: "org/repo",
      title: "edited",
      body: "keep",
    });
  },
);

it("retains drafts through a configuration failure and retries before creating", async () => {
  vi.mocked(fetch).mockRejectedValueOnce(new Error("offline"));
  const onsave = vi.fn();
  render(TaskForm, { title: "新規タスク", onsave, onclose: vi.fn() });
  await screen.findByText("実行先の設定を読み込めませんでした");
  await fireEvent.input(screen.getByLabelText("product"), {
    target: { value: "org/repo" },
  });
  await fireEvent.input(screen.getByLabelText("title"), {
    target: { value: "keep draft" },
  });
  await fireEvent.submit(document.querySelector("form")!);
  expect(onsave).not.toHaveBeenCalled();
  await fireEvent.click(
    screen.getByRole("button", { name: "実行先を再読み込み" }),
  );
  await screen.findByRole("option", { name: "forge" });
  expect((screen.getByLabelText("title") as HTMLInputElement).value).toBe(
    "keep draft",
  );
  await fireEvent.submit(document.querySelector("form")!);
  expect(onsave).toHaveBeenCalledWith(
    expect.objectContaining({ execution_target: "forge" }),
  );
});
it("a prefilled new task keeps the given fields and the default target", async () => {
  const onsave = vi.fn().mockResolvedValue(undefined);
  render(TaskForm, {
    title: "アイデアをタスク化",
    submitLabel: "タスクを作成",
    prefill: { product_id: "org/repo", title: "案", body: "本文" },
    onsave,
    onclose: vi.fn(),
  });
  await waitFor(() =>
    expect((screen.getByLabelText("実行先") as HTMLSelectElement).value).toBe(
      "forge",
    ),
  );
  await fireEvent.click(screen.getByRole("button", { name: "タスクを作成" }));
  expect(onsave).toHaveBeenCalledWith({
    product_id: "org/repo",
    title: "案",
    body: "本文",
    execution_target: "forge",
  });
});
