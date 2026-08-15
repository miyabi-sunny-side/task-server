import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { TaskCard as Task } from "./api";
import TaskCard from "./TaskCard.svelte";

const FIXTURE: Task = {
  id: "alpha",
  title: "Task Card 見本",
  body: "作業本文。CJK を含む。",
  status: "wip",
  kind: "normal",
  product_id: "sunny-side/task-server",
  priority: 0,
  branch: "task/alpha",
  claimed_by: "worker-1",
  claim_id: "claim-1",
  claimed_at: "2026-08-15T11:00:00Z",
  claim_expires_at: "2026-08-15T13:00:00Z",
  commit_sha: "abc1234def",
  verification: "cargo test --locked",
  release_tag: null,
  created_at: "2026-08-15T10:00:00Z",
  updated_at: "2026-08-15T12:00:00Z",
  available_transitions: ["ready", "blocked", "cancelled", "dropped"],
};

describe("TaskCard", () => {
  afterEach(cleanup);

  it("renders body, verification, commit, and one button per available transition", () => {
    render(TaskCard, { props: { task: FIXTURE } });

    expect(screen.getByText(FIXTURE.body)).toBeTruthy();
    expect(screen.getByText(FIXTURE.verification as string)).toBeTruthy();
    expect(screen.getByText(FIXTURE.commit_sha as string)).toBeTruthy();
    expect(screen.getByText(FIXTURE.status)).toBeTruthy();
    for (const status of FIXTURE.available_transitions) {
      expect(screen.getByRole("button", { name: status })).toBeTruthy();
    }
    expect(screen.getAllByRole("button")).toHaveLength(
      FIXTURE.available_transitions.length,
    );
  });

  it("marks only the ready transition as the primary button", () => {
    render(TaskCard, { props: { task: FIXTURE } });

    expect(
      screen
        .getByRole("button", { name: "ready" })
        .classList.contains("primary"),
    ).toBe(true);
    for (const status of ["blocked", "cancelled", "dropped"]) {
      expect(
        screen
          .getByRole("button", { name: status })
          .classList.contains("primary"),
      ).toBe(false);
    }
  });

  it("reports the chosen transition through ontransition", async () => {
    const ontransition = vi.fn();
    render(TaskCard, { props: { task: FIXTURE, ontransition } });

    await fireEvent.click(screen.getByRole("button", { name: "blocked" }));
    expect(ontransition).toHaveBeenCalledWith("blocked");

    await fireEvent.click(screen.getByRole("button", { name: "ready" }));
    expect(ontransition).toHaveBeenCalledWith("ready");
    expect(ontransition).toHaveBeenCalledTimes(2);
  });

  it("marks an instant:merge task and leaves a normal task unmarked", () => {
    render(TaskCard, {
      props: { task: { ...FIXTURE, kind: "instant:merge" } },
    });
    expect(screen.getByText("instant:merge")).toBeTruthy();

    cleanup();

    render(TaskCard, { props: { task: FIXTURE } });
    expect(screen.queryByText("instant:merge")).toBe(null);
  });

  it("disables every transition button while busy", () => {
    render(TaskCard, { props: { task: FIXTURE, busy: true } });

    for (const button of screen.getAllByRole("button")) {
      expect((button as HTMLButtonElement).disabled).toBe(true);
    }
  });
});
