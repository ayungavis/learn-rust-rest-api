FROM rust:1.94.1-bookworm AS build

WORKDIR /app

COPY Cargo.toml Cargo.lock build.rs ./
COPY migrations ./migrations
COPY src ./src

RUN --mount=type=cache,id=s/08257d5e-1865-4f75-94c5-10212993bbbe-/usr/local/cargo/registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=s/08257d5e-1865-4f75-94c5-10212993bbbe-/app/target,target=/app/target,sharing=locked \
    cargo build --locked --release --bin rust-catalog-api --jobs 1 \
    && cp /app/target/release/rust-catalog-api /tmp/rust-catalog-api

FROM debian:bookworm-slim AS runtime

RUN adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "10001" \
    appuser

WORKDIR /app

COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /tmp/rust-catalog-api /usr/local/bin/rust-catalog-api

ENV APP_ADDRESS=0.0.0.0:3000

EXPOSE 3000

USER appuser

STOPSIGNAL SIGINT

CMD ["rust-catalog-api"]
