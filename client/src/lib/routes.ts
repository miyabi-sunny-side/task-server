export interface RouteMatch {
  index: number;
  params: Record<string, string>;
}

interface RouteDef {
  pattern: RegExp;
  params: string[];
}

export const routes: RouteDef[] = [
  { pattern: /^\/$/, params: [] },
  { pattern: /^\/tasks\/([^/]+)$/, params: ["id"] },
  { pattern: /^\/closed$/, params: [] },
];

// The done page moved to /closed; the old address still reaches it (rewritten
// by syncRoute before matching, so it needs no route of its own).
export const REDIRECTS: Record<string, string> = { "/done": "/closed" };

export function matchRoute(pathname: string): RouteMatch {
  for (const [index, route] of routes.entries()) {
    const found = pathname.match(route.pattern);
    if (found) {
      const params: Record<string, string> = {};
      route.params.forEach((name, position) => {
        params[name] = decodeURIComponent(found[position + 1]);
      });
      return { index, params };
    }
  }
  return { index: 0, params: {} };
}
