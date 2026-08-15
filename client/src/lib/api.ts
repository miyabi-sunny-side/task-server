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
    throw new Error(`HTTP ${response.status}`);
  }
  return (await response.json()) as T;
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
  return requestJson(`/api/tasks/${encodeURIComponent(id)}/status`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ status }),
  });
}
