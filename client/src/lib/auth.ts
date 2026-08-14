export const CSRF_STORAGE_KEY = "task-server:csrf";

let sessionCsrf = "";

export function setSessionCsrf(token: string) {
  sessionCsrf = token;
  try {
    window.localStorage.setItem(CSRF_STORAGE_KEY, token);
  } catch {
    // ignore storage failures
  }
}

function storedCsrf(): string {
  if (sessionCsrf) {
    return sessionCsrf;
  }
  try {
    return window.localStorage.getItem(CSRF_STORAGE_KEY) || "";
  } catch {
    return "";
  }
}

export function authHeaders(): HeadersInit {
  const csrf = storedCsrf();
  if (!csrf) {
    return {};
  }
  return { "X-CSRF-Token": csrf };
}
