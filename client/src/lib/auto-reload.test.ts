import { describe, expect, it, vi } from "vitest";

import {
  AUTO_RELOAD_INTERVAL_MS,
  startAutoReload,
  type Timers,
} from "./auto-reload";

class FakeDocument extends EventTarget {
  visibilityState: DocumentVisibilityState = "visible";
}

function fakeTimers(): Timers {
  return {
    setInterval: vi.fn(setInterval) as unknown as typeof setInterval,
    clearInterval: vi.fn(clearInterval) as unknown as typeof clearInterval,
  };
}

function visibilityChange(documentRef: FakeDocument) {
  documentRef.dispatchEvent(new Event("visibilitychange"));
}

describe("startAutoReload", () => {
  it("does not call reload on start, and ticks it on the interval while visible", () => {
    vi.useFakeTimers();
    const documentRef = new FakeDocument();
    const timers = fakeTimers();
    const reload = vi.fn();

    startAutoReload(reload, { documentRef, timers });

    expect(reload).not.toHaveBeenCalled();
    vi.advanceTimersByTime(AUTO_RELOAD_INTERVAL_MS);
    expect(reload).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(AUTO_RELOAD_INTERVAL_MS);
    expect(reload).toHaveBeenCalledTimes(2);

    vi.useRealTimers();
  });

  it("reloads immediately and restarts the interval when the tab becomes visible", () => {
    vi.useFakeTimers();
    const documentRef = new FakeDocument();
    documentRef.visibilityState = "hidden";
    const timers = fakeTimers();
    const reload = vi.fn();

    startAutoReload(reload, { documentRef, timers });
    expect(reload).not.toHaveBeenCalled();
    vi.advanceTimersByTime(AUTO_RELOAD_INTERVAL_MS * 2);
    expect(reload).not.toHaveBeenCalled();

    documentRef.visibilityState = "visible";
    visibilityChange(documentRef);
    expect(reload).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(AUTO_RELOAD_INTERVAL_MS);
    expect(reload).toHaveBeenCalledTimes(2);

    vi.useRealTimers();
  });

  it("stops the interval when the tab becomes hidden", () => {
    vi.useFakeTimers();
    const documentRef = new FakeDocument();
    const timers = fakeTimers();
    const reload = vi.fn();

    startAutoReload(reload, { documentRef, timers });

    documentRef.visibilityState = "hidden";
    visibilityChange(documentRef);
    // Going hidden does not itself trigger a reload.
    expect(reload).not.toHaveBeenCalled();

    vi.advanceTimersByTime(AUTO_RELOAD_INTERVAL_MS * 3);
    expect(reload).not.toHaveBeenCalled();

    vi.useRealTimers();
  });

  it("removes the listener and clears the interval on cleanup", () => {
    vi.useFakeTimers();
    const documentRef = new FakeDocument();
    const timers = fakeTimers();
    const reload = vi.fn();

    const cleanup = startAutoReload(reload, { documentRef, timers });
    cleanup();

    vi.advanceTimersByTime(AUTO_RELOAD_INTERVAL_MS * 2);
    expect(reload).not.toHaveBeenCalled();

    documentRef.visibilityState = "hidden";
    visibilityChange(documentRef);
    documentRef.visibilityState = "visible";
    visibilityChange(documentRef);
    expect(reload).not.toHaveBeenCalled();

    vi.useRealTimers();
  });

  it("does not start an interval when the tab starts hidden", () => {
    vi.useFakeTimers();
    const documentRef = new FakeDocument();
    documentRef.visibilityState = "hidden";
    const timers = fakeTimers();
    const reload = vi.fn();

    startAutoReload(reload, { documentRef, timers });
    expect(timers.setInterval).not.toHaveBeenCalled();

    vi.useRealTimers();
  });

  it("uses the given intervalMs instead of the default", () => {
    vi.useFakeTimers();
    const documentRef = new FakeDocument();
    const timers = fakeTimers();
    const reload = vi.fn();

    startAutoReload(reload, { documentRef, timers, intervalMs: 5_000 });

    vi.advanceTimersByTime(5_000);
    expect(reload).toHaveBeenCalledTimes(1);

    vi.useRealTimers();
  });
});
