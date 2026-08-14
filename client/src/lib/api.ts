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
}

export interface TaskCard {
  id: string;
  title: string;
  status: string;
  body: string;
  verification: string | null;
  commit_sha: string | null;
  available_actions: string[];
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

export function fetchTasks(signal?: AbortSignal): Promise<TaskSummary[]> {
  return requestJson("/api/tasks", { signal });
}

export function fetchTask(id: string, signal?: AbortSignal): Promise<TaskCard> {
  return requestJson(`/api/tasks/${encodeURIComponent(id)}`, { signal });
}

export function postTaskAction(
  id: string,
  action: string,
  bump?: string,
): Promise<TaskCard> {
  return requestJson(
    `/api/tasks/${encodeURIComponent(id)}/actions/${encodeURIComponent(action)}`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(bump ? { bump } : {}),
    },
  );
}
