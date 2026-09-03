import { authHeaders, setSessionCsrf } from "./auth";

export interface Session {
  user: string;
  csrf_token: string;
}

export async function loadSession(signal?: AbortSignal): Promise<Session> {
  const session = await requestJson<Session>("/api/session", { signal });
  setSessionCsrf(session.csrf_token);
  return session;
}

export interface TaskSummary {
  id: string;
  title: string;
  status: string;
  kind: string;
  product_id: string;
  priority: number;
  updated_at: string;
  // The task this one waits for, so a list can say why a draft is waiting.
  depends_on?: string | null;
}

// What the latest *finished* review of a task said, read from that review's
// own row. A review still open has no verdict and does not answer here, so a
// task with nothing to show simply carries no `latest_review`.
export interface ReviewOutcome {
  review_task_id: string;
  verdict: "approve" | "request_changes";
  findings: string | null;
  subject_commit_sha: string | null;
  reported_at: string;
}

export interface TaskCard {
  id: string;
  title: string;
  body: string;
  status: string;
  kind: string;
  product_id: string;
  priority: number;
  branch: string | null;
  claimed_by: string | null;
  claim_id: string | null;
  claimed_at: string | null;
  claim_expires_at: string | null;
  commit_sha: string | null;
  verification: string | null;
  release_tag: string | null;
  created_at: string;
  updated_at: string;
  available_transitions: string[];
  latest_review?: ReviewOutcome;
  // How far shipping this work steps the version; on a release task, the
  // largest level it ships. Optional here only so fixtures written before the
  // field existed still type-check; the server always sends it.
  release_level?: "patch" | "minor" | "major";
  release_task_id?: string | null;
  // The task this one waits for; the landing of that task promotes this one.
  depends_on?: string | null;
  // What that task is doing while it has not landed. Absent once it has, or
  // when there is no dependency at all.
  dependency_status?: string;
}

// A refusal the server explained. The message is the human wording the
// operator reads; `code` is the stable slug a branch may test.
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

function refusal(status: number, body: string): ApiError {
  try {
    const parsed = JSON.parse(body) as { error?: unknown; code?: unknown };
    if (typeof parsed.error === "string" && parsed.error !== "") {
      const code = typeof parsed.code === "string" ? parsed.code : "";
      return new ApiError(status, code, parsed.error);
    }
  } catch {
    // not the documented error envelope; fall back to the status alone
  }
  return new ApiError(status, "", `HTTP ${status}`);
}

async function requestJson<T>(url: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  for (const [key, value] of Object.entries(authHeaders())) {
    if (!headers.has(key)) {
      headers.set(key, value);
    }
  }
  const response = await fetch(url, { ...init, headers });
  if (!response.ok) {
    throw refusal(response.status, await response.text());
  }
  return (await response.json()) as T;
}

function postJson<T>(url: string, body: unknown): Promise<T> {
  return requestJson(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function fetchTasks(
  signal?: AbortSignal,
  status?: string,
): Promise<TaskSummary[]> {
  const url = status
    ? `/api/tasks?status=${encodeURIComponent(status)}`
    : "/api/tasks";
  return requestJson(url, { signal });
}

export function fetchTask(id: string, signal?: AbortSignal): Promise<TaskCard> {
  return requestJson(`/api/tasks/${encodeURIComponent(id)}`, { signal });
}

export function postTaskStatus(id: string, status: string): Promise<TaskCard> {
  return postJson(`/api/tasks/${encodeURIComponent(id)}/status`, { status });
}

// Landed work no release is carrying, per product: the reconciliation window
// for releases. Empty while the automatic issuing works.
export interface Releasable {
  product_id: string;
  task_count: number;
}

// An outstanding merge, as /api/control reports it: the ordinary summary plus
// why this one stopped. A blocked merge holds up every merge of its product,
// so the reason travels with the queue rather than being fetched card by card
// — one payload, one generation, nothing to fail on its own.
export interface PendingMerge extends TaskSummary {
  // `null` while the merge is running: only a blocked merge has a reason.
  verification: string | null;
}

// An outstanding release, as /api/control reports it: the summary, how far it
// steps the version, and why it stopped — the same shape as a pending merge,
// read off the same payload.
export interface PendingRelease extends TaskSummary {
  release_level: "patch" | "minor" | "major";
  // `null` while the release is running: only a blocked one has a reason.
  verification: string | null;
}

// What the top page needs to draw the automated stretch of the pipeline. The
// `pending_*` lists are what the server is carrying; `mergeable`, `unreviewed`
// and `releasable` are the reconciliation windows, empty whenever the
// automatic issuing works. Nothing here is a button: the page asks the human
// for no decision.
export interface ControlPlane {
  mergeable: TaskSummary[];
  pending_merges: PendingMerge[];
  pending_releases: PendingRelease[];
  pending_reviews: TaskSummary[];
  unreviewed: TaskSummary[];
  releasable: Releasable[];
}

export function fetchControl(signal?: AbortSignal): Promise<ControlPlane> {
  return requestJson("/api/control", { signal });
}
