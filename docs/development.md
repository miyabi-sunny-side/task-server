# Development and releases

[Setup and first task](../README.md) · [Operations](operations.md) · [API](api.md) · [Workers](workers.md)

## Verification

See [AGENTS.md](../AGENTS.md) for build/test commands and [DESIGN.md](../DESIGN.md) for
UI behavior. Tests isolate data and agent execution from deployed work.

## Releases

Push a `vMAJOR.MINOR.PATCH` tag matching `Cargo.toml` and `Cargo.lock`.
This runs verification, builds the container, and smoke-tests it with an empty ledger.
The tested image is published as `ghcr.io/<owner>/<repo>:<version>` and `:latest`. The GitHub
Release records the image digest. Manual release runs must select a version tag.
PRs and manual CI runs perform verification; pushes to `main` do not prebuild or
publish images.

BuildKit retains intermediate stages at `ghcr.io/<owner>/<repo>:build-cache`
using the workflow's `GITHUB_TOKEN` with `packages: write`. This registry cache
works across release tags; no R2 credentials or separate cache service are needed.
An empty cache is valid. Cargo-chef separates dependencies from application code.
Its recipe masks the application's version so version bumps can reuse dependencies.
The real manifest supplies the compiled application's version. The frontend has an independent build stage.

The release profile uses Cargo's standard optimization and codegen settings,
with symbols stripped, to keep build time reasonable for this I/O-oriented server.
To run the same image check locally:

```sh
docker buildx build --load -t task-server:test .
bash .github/smoke-image.sh task-server:test
```

## Browser regression check

After building the frontend and `cargo build --locked`, run the isolated Chromium
check. It starts its own binary with temporary ledgers and external settings,
restarts that same binary with changed settings, and never connects to deployed tasks.
Playwright can stay outside the product's dependency tree:

```sh
task_browser_dir=$(mktemp -d)
npm install --prefix "$task_browser_dir" --no-save --package-lock=false playwright
"$task_browser_dir/node_modules/.bin/playwright" install chromium
PLAYWRIGHT_MODULE="$task_browser_dir/node_modules/playwright/index.mjs" node tests/execution-targets.e2e.mjs
rm -rf "$task_browser_dir"
```

Set `E2E_EVIDENCE_DIR` to a directory outside this repository to retain screenshots
and measurements. `TASK_SERVER_BINARY` can point at another built binary.
