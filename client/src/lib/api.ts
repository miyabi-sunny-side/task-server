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
  // Only a merge task carries one: the position the server will claim it in.
  merge_sequence: number | null;
  created_at: string;
  updated_at: string;
  available_transitions: string[];
  latest_review?: ReviewOutcome;
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

// What the top page needs to draw the automated stretch of the pipeline and
// decide whether release is a live button. The `pending_*` lists are what the
// server is carrying; `mergeable` and `unreviewed` are the reconciliation
// windows, empty whenever the automatic issuing works.
export interface ControlPlane {
  mergeable: TaskSummary[];
  pending_merges: PendingMerge[];
  pending_reviews: TaskSummary[];
  unreviewed: TaskSummary[];
  releasable: Releasable[];
}

export interface ReleaseResult {
  product_id: string;
  tag: string;
  released: TaskSummary[];
}

export function fetchControl(signal?: AbortSignal): Promise<ControlPlane> {
  return requestJson("/api/control", { signal });
}

export function postRelease(
  productId: string,
  tag: string,
): Promise<ReleaseResult> {
  return postJson("/api/releases", { product_id: productId, tag });
}
