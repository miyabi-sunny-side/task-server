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

  it("opens its head with the product, then the status and kind badges", () => {
    render(TaskCard, { props: { task: { ...FIXTURE, kind: "review" } } });

    const meta = document.querySelector<HTMLElement>(".meta")!;
    const product = meta.querySelector<HTMLElement>(".product")!;
    expect(meta.firstElementChild).toBe(product);
    expect(product.textContent?.trim()).toBe(FIXTURE.product_id);
    const badges = [...meta.querySelectorAll<HTMLElement>(".badge")].map(
      (badge) => badge.textContent?.trim(),
    );
    expect(badges).toEqual(["wip", "review"]);
    expect(
      product.compareDocumentPosition(meta.querySelector(".badge")!) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("names any kind that is not normal in a badge, and leaves a normal task unmarked", () => {
    for (const kind of ["instant:merge", "review"]) {
      render(TaskCard, { props: { task: { ...FIXTURE, kind } } });
      expect(screen.getByText(kind)).toBeTruthy();
      cleanup();
    }

    render(TaskCard, { props: { task: FIXTURE } });
    expect(screen.queryByText("instant:merge")).toBe(null);
    expect(screen.queryByText("review")).toBe(null);
    expect(document.querySelectorAll(".badge")).toHaveLength(1);
  });

  it("shows a review task's subject commit as a muted caption", () => {
    render(TaskCard, {
      props: { task: { ...FIXTURE, kind: "review", commit_sha: "9f8e7d6c" } },
    });

    const caption = document.querySelector<HTMLElement>(
      '[data-field="subject_commit_sha"]',
    );
    expect(caption).not.toBeNull();
    expect(caption!.classList.contains("caption")).toBe(true);
    expect(caption!.textContent).toContain("9f8e7d6c");

    cleanup();

    render(TaskCard, { props: { task: FIXTURE } });
    expect(
      document.querySelector('[data-field="subject_commit_sha"]'),
    ).toBeNull();
  });

  it("renders the review block before the body, verdict and findings included", () => {
    render(TaskCard, {
      props: {
        task: {
          ...FIXTURE,
          latest_review: {
            review_task_id: "rev-1",
            verdict: "request_changes",
            findings: "境界値が抜けています。\n再提出してください。",
            subject_commit_sha: "abc1234def",
            reported_at: "2026-08-16T09:00:00Z",
          },
        },
      },
    });

    const block = document.querySelector<HTMLElement>(
      '[data-field="latest_review"]',
    );
    expect(block).not.toBeNull();
    expect(block!.textContent).toContain("レビュー");
    expect(screen.getByText("request_changes").classList).toContain("badge");

    const findings = block!.querySelector<HTMLElement>("[data-findings]");
    expect(findings?.textContent).toBe(
      "境界値が抜けています。\n再提出してください。",
    );

    const body = screen.getByText(FIXTURE.body);
    expect(
      block!.compareDocumentPosition(body) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("renders no review block at all for a task no review has answered", () => {
    render(TaskCard, { props: { task: FIXTURE } });

    expect(document.querySelector('[data-field="latest_review"]')).toBeNull();
    expect(screen.queryByText("レビュー")).toBe(null);
  });

  it("disables every transition button while busy", () => {
    render(TaskCard, { props: { task: FIXTURE, busy: true } });

    for (const button of screen.getAllByRole("button")) {
      expect((button as HTMLButtonElement).disabled).toBe(true);
    }
  });

  it("names the dependency it waits for, with its status until it lands", () => {
    render(TaskCard, {
      props: {
        task: { ...FIXTURE, depends_on: "beta", dependency_status: "wip" },
      },
    });

    const field = document.querySelector<HTMLElement>(
      '[data-field="depends_on"]',
    )!;
    expect(field.textContent).toContain("beta");
    expect(field.querySelector("a")?.getAttribute("href")).toBe("/tasks/beta");
    expect(
      field.querySelector("[data-dependency-status]")?.textContent?.trim(),
    ).toBe("wip");

    cleanup();
    render(TaskCard, { props: { task: { ...FIXTURE, depends_on: "beta" } } });
    expect(
      document.querySelector(
        '[data-field="depends_on"] [data-dependency-status]',
      ),
    ).toBeNull();

    cleanup();
    render(TaskCard, { props: { task: FIXTURE } });
    expect(document.querySelector('[data-field="depends_on"]')).toBeNull();
  });

  it("wears who blocked it beside the status when blocked", () => {
    render(TaskCard, {
      props: {
        task: { ...FIXTURE, status: "blocked", blocked_by: "operator" },
      },
    });
    const badges = [...document.querySelectorAll<HTMLElement>(".badge")].map(
      (badge) => badge.textContent?.trim(),
    );
    expect(badges.slice(0, 2)).toEqual(["blocked", "保留"]);
  });
});
