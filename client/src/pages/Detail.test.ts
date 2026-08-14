import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import Detail from "./Detail.svelte";

const CARD = {
  id: "alpha",
  title: "見本タスク",
  status: "awaiting_user",
  body: "詳細本文",
  verification: "cargo test",
  commit_sha: "abc1234",
  available_actions: ["done", "push"],
};

const DONE_CARD = {
  ...CARD,
  status: "done",
  available_actions: [] as string[],
};

describe("Detail", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("loads a task card and posts an action then reloads", async () => {
    let current = CARD;
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockImplementation((input, init) => {
        if (String(input).includes("/actions/") && init?.method === "POST") {
          current = DONE_CARD;
          return Promise.resolve(
            new Response(JSON.stringify(DONE_CARD), { status: 200 }),
          );
        }
        return Promise.resolve(
          new Response(JSON.stringify(current), { status: 200 }),
        );
      });
    vi.stubGlobal("fetch", fetchMock);

    render(Detail, { props: { id: "alpha" } });

    await waitFor(() =>
      expect(screen.getByRole("heading", { name: CARD.title })).toBeTruthy(),
    );
    expect(screen.getByText(CARD.body)).toBeTruthy();
    expect(screen.getByText(CARD.verification)).toBeTruthy();
    expect(screen.getByText(CARD.commit_sha)).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "done" }));
    await waitFor(() => expect(screen.getByText("done")).toBeTruthy());
    expect(
      fetchMock.mock.calls.some(
        ([input, init]) =>
          String(input).includes("/actions/done") && init?.method === "POST",
      ),
    ).toBe(true);
  });
});
