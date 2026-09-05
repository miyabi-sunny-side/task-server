# syntax=docker/dockerfile:1

FROM node:24-bookworm-slim AS frontend
WORKDIR /app/client
COPY client/package.json client/package-lock.json ./
RUN npm ci
COPY client/ ./
RUN npm run build

# cargo-chef splits the Rust build into a dependency layer (cooked from the
# recipe, which masks the local package version) and the crate's own build.
# Source edits and release version bumps therefore reuse compiled dependencies.
FROM rust:1.96-bookworm AS chef
WORKDIR /app
COPY rust-toolchain.toml ./
RUN cargo install cargo-chef --version 0.1.78 --locked

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS backend
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --locked --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime
WORKDIR /app
COPY --from=backend /app/target/release/task-server /usr/local/bin/task-server
COPY --from=frontend /app/client/dist ./client/dist
# Markdown records live below APP_DATA_DIR; the runtime user owns the directory.
RUN install -d -o 10001 -g 10001 /app/data
VOLUME ["/app/data"]
ENV APP_BIND_ADDR=0.0.0.0:3000
ENV APP_DATA_DIR=/app/data/ledger
EXPOSE 3000
USER 10001:10001
ENTRYPOINT ["task-server"]
