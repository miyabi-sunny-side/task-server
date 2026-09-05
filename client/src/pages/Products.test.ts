import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import Products from "./Products.svelte";

const products = [true, false, null].map((releases, index) => ({
  id: `org/product-${index}`,
  repository: `https://github.com/org/product-${index}`,
  description: `製品説明 ${index}\n次の行`,
  local_path: index === 0 ? "/projects/org/product-0" : null,
  releases,
  archived: index === 2,
  archived_at: index === 2 ? "2026-09-06T01:02:03Z" : null,
}));

function response(payload: unknown, status = 200) {
  return new Response(JSON.stringify(payload), { status });
}

function region() {
  return screen.getByRole("region", { name: "プロダクト一覧" });
}

function refresh() {
  document.dispatchEvent(new Event("visibilitychange"));
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

it("shows all registered metadata and distinguishes false from unset releases", async () => {
  const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(response(products));
  vi.stubGlobal("fetch", fetchMock);
  render(Products);
  await waitFor(() => expect(region().dataset.state).toBe("success"));
  expect(fetchMock.mock.calls[0][0]).toBe("/api/products");
  const rows = screen.getAllByRole("listitem");
  expect(rows).toHaveLength(3);
  for (const [index, row] of rows.entries()) {
    expect(row.textContent).toContain(products[index].id);
    expect(row.textContent).toContain(products[index].repository);
    expect(row.textContent).toContain(`製品説明 ${index}`);
    expect(row.textContent).toContain(["公開", "公開しない", "未設定"][index]);
  }
  expect(rows[0].textContent).toContain(products[0].local_path);
  expect(rows[2].textContent).toContain("アーカイブ済み");
  expect(rows[2].textContent).toContain(products[2].archived_at);
  expect(within(region()).queryByRole("button")).toBeNull();
});

it("shows loading and error, retries to empty, then refreshes to populated", async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValueOnce(response([]))
    .mockResolvedValue(response(products));
  vi.stubGlobal("fetch", fetchMock);
  render(Products);
  expect(region().dataset.state).toBe("loading");
  await screen.findByText("読み込みに失敗しました");
  expect(region().dataset.state).toBe("error");
  await fireEvent.click(screen.getByRole("button", { name: "再試行" }));
  await screen.findByText("登録済みのプロダクトがありません");
  expect(region().dataset.state).toBe("empty");
  refresh();
  await waitFor(() => expect(region().dataset.state).toBe("success"));
});

it("retains drawn metadata during failed background reloads and stops on unmount", async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(response(products))
    .mockRejectedValue(new Error("offline"));
  vi.stubGlobal("fetch", fetchMock);
  const { unmount } = render(Products);
  await waitFor(() => expect(region().dataset.state).toBe("success"));
  const content = region().textContent;
  refresh();
  await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
  expect(region().dataset.state).toBe("success");
  expect(region().textContent).toBe(content);
  const signal = fetchMock.mock.calls[1][1]?.signal;
  unmount();
  expect(signal?.aborted).toBe(true);
  refresh();
  expect(fetchMock).toHaveBeenCalledTimes(2);
});
