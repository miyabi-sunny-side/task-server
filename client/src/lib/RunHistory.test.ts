import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import RunHistory from "./RunHistory.svelte";
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
it("loads only when opened, retries errors, and pages without marking read", async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          runs: [
            {
              id: 1,
              at: "2026-09-05",
              source: "worker",
              outcome: "blocked",
              note: "依存待ち",
            },
          ],
          next: 1,
        }),
      ),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          runs: [
            { id: 2, at: "2026-09-06", source: "worker", outcome: "done" },
          ],
          next: null,
        }),
      ),
    );
  vi.stubGlobal("fetch", fetchMock);
  render(RunHistory, { taskId: "one" });
  expect(fetchMock).not.toHaveBeenCalled();
  const details = document.querySelector("details")!;
  details.open = true;
  await fireEvent(details, new Event("toggle"));
  await screen.findByText("実行履歴の読み込みに失敗しました");
  await fireEvent.click(screen.getByRole("button", { name: "再試行" }));
  await screen.findByText(/2026-09-05 · blocked/);
  await fireEvent.click(screen.getByRole("button", { name: "続きを読み込む" }));
  await screen.findByText(/2026-09-06 · done/);
  expect(fetchMock.mock.calls[2][0]).toBe("/api/runs?task_id=one&since=1");
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "続きを読み込む" })).toBeNull(),
  );
  expect(
    fetchMock.mock.calls.every(
      ([, init]) => !init?.method || init.method === "GET",
    ),
  ).toBe(true);
});
