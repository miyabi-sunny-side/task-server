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
  // Index 2 is the retired /done address. syncRoute rewrites it to /closed
  // before matching, so it never renders; it stays so the indexes above and
  // below it keep their meaning.
  { pattern: /^\/done$/, params: [] },
  { pattern: /^\/closed$/, params: [] },
];

// The done page moved to /closed; the old address still reaches it.
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
