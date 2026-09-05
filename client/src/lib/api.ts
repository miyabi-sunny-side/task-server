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

export interface Milestone {
  report_id?: number;
  name: "implemented" | "verified" | "reviewed" | "merged" | "released";
  at: string;
  commit_sha?: string | null;
  evidence?: string | null;
}

export interface TaskSummary {
  archived?: boolean;
  id: string;
  title: string;
  status: string;
  kind: string;
  product_id: string;
  priority: number;
  updated_at: string;
  // The task this one waits for, and what it is doing while it has not
  // landed, so a list can say a ready task is waiting its turn. The status is
  // absent once the dependency landed, or without a dependency.
  depends_on?: string | null;
  dependency_status?: string;
  // Who put a `blocked` task there: a person parking it ("operator"), a
  // worker's report ("worker"), or the control plane ("system").
  blocked_by?: BlockedBy | null;
}

export type BlockedBy = "operator" | "worker" | "system";

// The word a badge wears for who blocked a task. A parked task says so in the
// operator's own language; the other two name the actor.
export function blockedByLabel(by: BlockedBy): string {
  return by === "operator" ? "保留" : by;
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

// Ledger reports retain both textual evidence and structured command results.
export type Check = string | { name: string; exit_code: number };

export function checkLabel(check: Check): string {
  return typeof check === "string"
    ? check
    : `${check.name}: exit ${check.exit_code}`;
}

export interface TaskCard {
  report_id?: number;
  report_ids?: number[];
  legacy_completion?: {
    at?: string;
    commit_sha?: string;
    summary?: string;
    verification?: string;
    checks?: Check[];
  }[];
  archived?: boolean;
  milestones?: Milestone[];
  milestone_history?: Milestone[];
  runs_count?: number;
  runs_unread?: number;
  id: string;
  title: string;
  body: string;
  status: string;
  blocked_by?: BlockedBy | null;
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
  // One or two sentences a person reads first; the log lives in verification.
  summary?: string | null;
  // The checks the worker ran, as reported.
  checks?: Check[];
  // How far shipping this work steps the version; on a release task, the
  // largest level it ships. Optional here only so fixtures written before the
  // field existed still type-check; the server always sends it.
  release_level?: "patch" | "minor" | "major";
  release_task_id?: string | null;
  // The task this one waits for; no claim hands this one out before it lands.
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

// A row of the done screen: what a `normal` task finished, and when.
// `done_at` is the moment this task first reached `done` — not `updated_at`,
// which keeps moving through approval, landing, and release.
export interface DoneTask {
  id: string;
  title: string;
  status: string;
  product_id: string | null;
  release_tag: string | null;
  verification: string | null;
  // The completion report a person reads; `verification` is the log behind it.
  summary?: string | null;
  done_at: string | null;
}

export function fetchDone(signal?: AbortSignal): Promise<DoneTask[]> {
  return requestJson("/api/done", { signal });
}

// A row of the closed screen: finished `normal` work and `normal` work that was
// called off, in one list. `closed_at` is the moment it closed (the server's
// sort key: `done_at` for finished work, the cancelling for the rest).
export interface ClosedTask extends DoneTask {
  archived?: boolean;
  closed_at: string;
}

export function fetchClosed(signal?: AbortSignal): Promise<ClosedTask[]> {
  return requestJson("/api/closed", { signal });
}

export function fetchTask(id: string, signal?: AbortSignal): Promise<TaskCard> {
  return requestJson(`/api/tasks/${encodeURIComponent(id)}`, { signal });
}

export function postTaskStatus(id: string, status: string): Promise<TaskCard> {
  return postJson(`/api/tasks/${encodeURIComponent(id)}/status`, { status });
}

// Compatibility fields are accepted but never rendered as active pipeline work.
export type StuckReason =
  | "unclaimed"
  | "lease-expired"
  | "no-subtask"
  | "subtask-unclaimed"
  | "blocked"
  | "release-stalled";

export interface Stuck {
  task_id: string;
  kind: string;
  status: string;
  since: string;
  reason: StuckReason;
}

export interface ControlPlane {
  mergeable: TaskSummary[];
  pending_merges: (TaskSummary & { verification: string | null })[];
  pending_releases: (TaskSummary & {
    verification: string | null;
    release_level: "patch" | "minor" | "major";
  })[];
  pending_reviews: TaskSummary[];
  unreviewed: TaskSummary[];
  releasable: { product_id: string; task_count: number }[];
  stuck: Stuck[];
}

export function fetchControl(signal?: AbortSignal): Promise<ControlPlane> {
  return requestJson("/api/control", { signal });
}

export interface TaskFields {
  title: string;
  product_id: string;
  body: string;
}

export function createTask(fields: TaskFields): Promise<TaskCard> {
  return postJson("/api/tasks", fields);
}

export function updateTask(id: string, fields: TaskFields): Promise<TaskCard> {
  return requestJson(`/api/tasks/${encodeURIComponent(id)}`, {
    method: "PATCH",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(fields),
  });
}

export interface Run {
  body?: string;
  claim_id?: string;
  task_id?: string;
  id: number;
  at: string;
  source: string;
  outcome?: string | null;
  worker?: string | null;
  model?: string | null;
  commit_sha?: string | null;
  agent_secs?: number | null;
  note?: string | null;
  stdout_tail?: string | null;
  stderr_tail?: string | null;
  checks?: Check[];
  read_at?: string | null;
}

export function fetchRuns(
  taskId: string,
  since = 0,
): Promise<{ runs: Run[]; next: number | null }> {
  return requestJson(
    `/api/runs?task_id=${encodeURIComponent(taskId)}&since=${since}`,
  );
}

export function fetchRun(id: number): Promise<Run> {
  return requestJson(`/api/runs/${id}`);
}
