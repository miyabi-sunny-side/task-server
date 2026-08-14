import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import Home from "./Home.svelte";

const TASKS = [
  {
    id: "theme",
    title: "テーマ切替",
    status: "ready",
  },
  {
    id: "router",
    title: "ルーター",
    status: "awaiting_user",
  },
];

function jsonResponse(payload: unknown): Response {
  return new Response(JSON.stringify(payload), { status: 200 });
}

function listContainer(): HTMLElement {
  const container = document.querySelector<HTMLElement>("[data-state]");
  if (!container) {
    throw new Error("list container with data-state was not found");
  }
  return container;
}

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: state,
  });
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("Home", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("loads tasks on display and renders them as card links", async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse(TASKS));
    vi.stubGlobal("fetch", fetchMock);

    render(Home);
    expect(listContainer().dataset.state).toBe("loading");

    await waitFor(() => expect(listContainer().dataset.state).toBe("success"));
    const card = screen.getByRole("link", { name: /テーマ切替/ });
    expect(card.getAttribute("href")).toBe("/tasks/theme");
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("reloads the list when the tab becomes visible again", async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse(TASKS));
    vi.stubGlobal("fetch", fetchMock);

    render(Home);
    await waitFor(() => expect(listContainer().dataset.state).toBe("success"));

    setVisibility("hidden");
    expect(fetchMock).toHaveBeenCalledTimes(1);

    setVisibility("visible");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
  });

  it("shows the empty state for an empty list", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>().mockResolvedValue(jsonResponse([])),
    );

    render(Home);

    await waitFor(() => expect(listContainer().dataset.state).toBe("empty"));
  });

  it("shows the error state and recovers through the retry button", async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce(jsonResponse(TASKS));
    vi.stubGlobal("fetch", fetchMock);

    render(Home);
    await waitFor(() => expect(listContainer().dataset.state).toBe("error"));

    await fireEvent.click(screen.getByRole("button", { name: "再試行" }));
    await waitFor(() => expect(listContainer().dataset.state).toBe("success"));
  });
});
