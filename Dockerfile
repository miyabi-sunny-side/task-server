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
# actions.rs が include_str! で参照するため build 時に必要
COPY config/ config/
RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime
WORKDIR /app
COPY --from=backend /app/target/release/task-server /usr/local/bin/task-server
COPY --from=frontend /app/client/dist ./client/dist
ENV APP_BIND_ADDR=0.0.0.0:3000
EXPOSE 3000
USER 10001:10001
ENTRYPOINT ["task-server"]
