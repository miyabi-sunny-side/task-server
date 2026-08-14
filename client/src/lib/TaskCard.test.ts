import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { TaskCard as Task } from "./api";
import TaskCard from "./TaskCard.svelte";

const FIXTURE: Task = {
  id: "alpha",
  title: "Task Card 見本",
  status: "awaiting_user",
  body: "作業本文。CJK を含む。",
  verification: "cargo test --locked",
  commit_sha: "abc1234def",
  available_actions: ["done", "push", "bump-tag", "ask-more"],
};

describe("TaskCard", () => {
  afterEach(cleanup);

  it("renders body, verification, commit, and action buttons from props", () => {
    render(TaskCard, { props: { task: FIXTURE } });

    expect(screen.getByText(FIXTURE.body)).toBeTruthy();
    expect(screen.getByText(FIXTURE.verification as string)).toBeTruthy();
    expect(screen.getByText(FIXTURE.commit_sha as string)).toBeTruthy();
    expect(screen.getByText(FIXTURE.status)).toBeTruthy();
    expect(screen.getByRole("button", { name: "done" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "push" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "ask-more" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "bump-tag patch" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "bump-tag minor" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "bump-tag major" })).toBeTruthy();
  });

  it("posts bump-tag with the chosen bump through onaction", async () => {
    const onaction = vi.fn();
    render(TaskCard, { props: { task: FIXTURE, onaction } });

    await fireEvent.click(
      screen.getByRole("button", { name: "bump-tag minor" }),
    );
    expect(onaction).toHaveBeenCalledWith("bump-tag", "minor");

    await fireEvent.click(screen.getByRole("button", { name: "done" }));
    expect(onaction).toHaveBeenCalledWith("done", undefined);
  });
});
