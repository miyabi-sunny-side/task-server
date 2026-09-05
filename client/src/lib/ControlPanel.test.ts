import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import ControlPanel from "./ControlPanel.svelte";
import type { ControlPlane, TaskSummary } from "./api";
const task: TaskSummary = {
  id: "held",
  title: "保留中の作業",
  product_id: "sunny-side/task-server",
  status: "blocked",
  kind: "normal",
  priority: 0,
  updated_at: "2026-09-05",
};
const empty: ControlPlane = {
  mergeable: [],
  unreviewed: [],
  releasable: [],
  pending_merges: [],
  pending_reviews: [],
  pending_releases: [],
  stuck: [],
};
afterEach(cleanup);
it("ignores retired automatic pipeline fields and alarms", () => {
  render(ControlPanel, {
    fetchState: "ready",
    plane: {
      ...empty,
      pending_reviews: [task],
      mergeable: [task],
      unreviewed: [task],
      releasable: [{ product_id: task.product_id, task_count: 1 }],
      stuck: [
        {
          task_id: task.id,
          status: "done",
          kind: "normal",
          since: "2026-09-05",
          reason: "no-subtask",
        },
      ],
    },
  });
  expect(
    document.querySelector('[data-region="control"]')?.textContent?.trim(),
  ).toBe("");
  expect(screen.queryByRole("status")).toBeNull();
});
it("shows blocked and expired executions once as neutral task links", () => {
  render(ControlPanel, {
    fetchState: "ready",
    tasks: [task],
    plane: {
      ...empty,
      stuck: [
        {
          task_id: task.id,
          status: task.status,
          kind: task.kind,
          since: "2026-09-05",
          reason: "blocked",
        },
      ],
    },
  });
  expect(screen.getByRole("status")).toBeTruthy();
  expect(screen.getByText("実行が止まっています")).toBeTruthy();
  expect(screen.getAllByRole("link")).toHaveLength(1);
  expect(screen.getByRole("link").textContent).toContain(task.title);
  expect(screen.queryByRole("button")).toBeNull();
});
it("keeps loading and failed-fetch retry observable", async () => {
  const onretry = vi.fn();
  const { rerender } = render(ControlPanel, { fetchState: "loading", onretry });
  expect(document.querySelector(".spinner")).toBeTruthy();
  await rerender({ fetchState: "error", onretry });
  await fireEvent.click(screen.getByRole("button", { name: "再試行" }));
  expect(onretry).toHaveBeenCalledOnce();
});
