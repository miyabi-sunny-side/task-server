# task-server

A task ledger with a browser UI, HTTP API, and MCP access.
Tasks, product metadata, execution notes, and history are stored as Markdown files.
An optional worker loop runs a fresh agent context for each task.

## Run with Docker

Requires Docker. In an empty directory, create your execution-target configuration:

```sh
cat > execution-targets.yaml <<'YAML'
labels:
  - local
default: local
YAML
chmod 644 execution-targets.yaml
```

Execution targets name the queues where work should run; they do not launch a worker.
Choose your own names. Without this configuration, new tasks have no destination.
The server reads the file once at startup; restart after changing it.

Start the published image with persistent storage:

```sh
docker run --name task-server \
  -p 127.0.0.1:3000:3000 \
  -v task-server-data:/app/data \
  -v "$PWD/execution-targets.yaml:/etc/task-server/execution-targets.yaml:ro" \
  -e EXECUTION_TARGETS_FILE=/etc/task-server/execution-targets.yaml \
  ghcr.io/miyabi-sunny-side/task-server:latest
```

Open `http://127.0.0.1:3000`. Data lives in the `task-server-data` volume.
Keep that volume when replacing the container.
The configuration file must be readable by container UID/GID `10001:10001`.

The Docker example publishes only on localhost. Native runs listen on all IPv4 interfaces.
HTTP and MCP have no built-in authentication. Control access at your network or proxy.
See [configuration and backups](docs/operations.md) before sharing or upgrading a ledger.

## Create your first task

Use the browser's task creation form, or run:

```sh
curl --fail-with-body http://127.0.0.1:3000/api/tasks \
  -H 'Content-Type: application/json' \
  -d '{"id":"first-task","title":"Review project documentation","body":"Describe the desired result and completion criteria.","execution_target":"local"}'
```

The response is the new task in `draft`; it also appears in the browser.
Use a new ID for another task. You can edit the title and body before making it ready.

First, [register a product](docs/api.md#explicit-product-metadata) to queue development work.
Supply its repository and release policy, then assign it to the task.
Set the task to `ready` when its scope is settled.
A [worker](docs/workers.md) must claim it to start execution; the server does not launch agents.
Failed or interrupted work remains `blocked`. Its history is available for resumption.

## Run from source

Requires Git, Rust 1.96+, and Node.js 24 with npm.
Create `execution-targets.yaml` outside the checkout as above, then build:

```sh
git clone https://github.com/miyabi-sunny-side/task-server.git
cd task-server
npm --prefix client ci
npm --prefix client run build
EXECUTION_TARGETS_FILE=/absolute/path/to/execution-targets.yaml \
  APP_DATA_DIR=data/ledger cargo run --locked
```

Replace the configuration path with your file's absolute path.
The ledger directory is created automatically. Keep it between runs.
The Rust binary embeds the frontend; rebuild both after frontend changes.
For a standalone binary, run `cargo build --locked --release` after the frontend build.
Copy `target/release/task-server`; no separate UI directory is needed at runtime.

## Reference

- [Operations](docs/operations.md): configuration, target names, storage, migration, and backups.
- [HTTP and MCP](docs/api.md): product registration, task reads and mutations, ideas, execution notes, pagination.
- [Worker execution](docs/workers.md): claims, leases, checkpoints, completion reports, and resumption.
- [Development and releases](docs/development.md): build checks, browser verification, and container publishing.

MCP is served at `http://127.0.0.1:3000/mcp`.
The same ledger backs the browser, HTTP API, and MCP tools.

Licensed under [MIT](LICENSE).
