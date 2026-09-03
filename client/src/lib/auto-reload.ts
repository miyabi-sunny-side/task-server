export const AUTO_RELOAD_INTERVAL_MS = 30_000;

export interface Timers {
  setInterval: typeof setInterval;
  clearInterval: typeof clearInterval;
}

// Only the sliver of Document this helper touches, so a test double doesn't
// have to impersonate the whole interface.
export interface VisibilityDocument {
  readonly visibilityState: DocumentVisibilityState;
  addEventListener(type: "visibilitychange", listener: () => void): void;
  removeEventListener(type: "visibilitychange", listener: () => void): void;
}

export interface AutoReloadOptions {
  documentRef?: VisibilityDocument;
  timers?: Timers;
  intervalMs?: number;
}

// Reload while the page is visible, on the same rhythm as a worker tick.
//
// The initial load remains the caller's responsibility. Returning to a
// visible tab reloads immediately, then starts a fresh interval.
export function startAutoReload(
  reload: () => void,
  {
    documentRef = document,
    timers = globalThis,
    intervalMs = AUTO_RELOAD_INTERVAL_MS,
  }: AutoReloadOptions = {},
): () => void {
  let intervalId: ReturnType<typeof setInterval> | null = null;

  function stopInterval() {
    if (intervalId === null) return;
    timers.clearInterval(intervalId);
    intervalId = null;
  }

  function startInterval() {
    stopInterval();
    if (documentRef.visibilityState !== "visible") return;
    intervalId = timers.setInterval(() => reload(), intervalMs);
  }

  function handleVisibilityChange() {
    if (documentRef.visibilityState !== "visible") {
      stopInterval();
      return;
    }
    reload();
    startInterval();
  }

  documentRef.addEventListener("visibilitychange", handleVisibilityChange);
  startInterval();

  return () => {
    stopInterval();
    documentRef.removeEventListener("visibilitychange", handleVisibilityChange);
  };
}
