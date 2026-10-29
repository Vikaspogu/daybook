# web UI -> dist (rust-embed embeds it into the binary at compile time)
FROM node:22-alpine AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# bookworm builder = glibc 2.36, matching the distroless cc-debian12 runtime
# (unqualified -rust-1 tags moved to trixie/glibc 2.41 and broke the runtime)
FROM lukemathwalker/cargo-chef:latest-rust-1-bookworm AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
COPY --from=web /web/dist web/dist
RUN cargo build --release --bin daybook

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /app/target/release/daybook /usr/local/bin/daybook
EXPOSE 31457
# config.toml (server port, push token list) is mounted at /config/config.toml
ENTRYPOINT ["daybook"]
CMD ["serve", "--config", "/config/config.toml"]
