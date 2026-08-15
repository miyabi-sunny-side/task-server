# syntax=docker/dockerfile:1

FROM node:24-bookworm-slim AS frontend
WORKDIR /app/client
COPY client/package.json client/package-lock.json ./
RUN npm ci
COPY client/ ./
RUN npm run build

FROM rust:1.96-bookworm AS backend
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src/ src/
RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime
WORKDIR /app
COPY --from=backend /app/target/release/task-server /usr/local/bin/task-server
COPY --from=frontend /app/client/dist ./client/dist
# The sqlite database is created at APP_DB_PATH on first start, so the
# unprivileged runtime user needs a writable directory for it.
RUN install -d -o 10001 -g 10001 /app/data
VOLUME ["/app/data"]
ENV APP_BIND_ADDR=0.0.0.0:3000
ENV APP_DB_PATH=/app/data/task-server.db
EXPOSE 3000
USER 10001:10001
ENTRYPOINT ["task-server"]
