# Rust Catalog API

A REST API built with Rust and Axum for authentication, user profiles, and a product catalog with image uploads to Cloudflare R2.

## Features

- Registration, email confirmation, login, logout, and password reset
- User profile and password management
- Public product reads and owner-only product writes
- Product image uploads to Cloudflare R2
- Consistent JSON errors with request IDs
- OpenAPI documentation with Swagger UI
- Liveness and readiness checks
- Request logging, timeouts, and graceful shutdown
- Unit and HTTP integration tests
- Multi-stage, non-root Docker image

## Technology

- Axum and Tokio
- PostgreSQL and SQLx
- Cloudflare R2 through the AWS SDK for Rust
- Lettre for SMTP email
- Utoipa and Swagger UI
- `tracing` and Tower HTTP middleware

## Requirements

- Rust 1.94.1 or newer
- PostgreSQL
- An SMTP server; Mailpit is suitable for local development
- A Cloudflare R2 bucket with S3 API credentials and public access configured
- GNU Make
- Docker, only when running the container workflow

## Local setup

1. Create the local environment file:

   ```bash
   cp .env.example .env
   ```

2. Replace every placeholder in `.env` with valid local or Cloudflare values.

   | Variable               | Purpose                                                 |
   | ---------------------- | ------------------------------------------------------- |
   | `APP_ADDRESS`          | HTTP bind address, normally `0.0.0.0:3000`              |
   | `DATABASE_URL`         | PostgreSQL connection URL                               |
   | `SMTP_URL`             | SMTP server URL                                         |
   | `FRONTEND_URL`         | Frontend origin allowed by CORS and used in email links |
   | `MAIL_FROM`            | Sender name and email address                           |
   | `R2_ACCOUNT_ID`        | Cloudflare account ID                                   |
   | `R2_ACCESS_KEY_ID`     | R2 access key ID                                        |
   | `R2_SECRET_ACCESS_KEY` | R2 secret access key                                    |
   | `R2_BUCKET`            | R2 bucket name                                          |
   | `R2_PUBLIC_BASE_URL`   | Public bucket or custom-domain base URL                 |

3. Ensure PostgreSQL, SMTP, and R2 are reachable with those values.

4. Start the API:

   ```bash
   make dev
   ```

   Database migrations run automatically during startup. A successful startup logs `API listening`.

5. Verify the application:

   ```bash
   curl -i http://localhost:3000/api/v1/health/live
   curl -i http://localhost:3000/api/v1/health/ready
   ```

   Both endpoints should return HTTP `200`. The JSON bodies are `{"status":"ok"}` and `{"status":"ready"}`.

## API documentation

- Swagger UI: <http://localhost:3000/docs>
- OpenAPI JSON: <http://localhost:3000/api-docs/openapi.json>

Swagger is the source of truth for endpoint paths, request bodies, response schemas, and authentication requirements.

Public product reads do not require authentication. Protected endpoints use an opaque session token:

```http
Authorization: Bearer <access_token>
```

## Error contract

Application errors use one JSON shape across the API. For example, an unknown route returns:

```json
{
  "code": "ROUTE_NOT_FOUND",
  "message": "The requested route does not exist",
  "request_id": "019..."
}
```

The same request ID is returned in the `x-request-id` response header. Include it in frontend error reports so a failed request can be investigated. Validation errors can additionally include a `details` array.

## Quality checks

Run the complete local validation:

```bash
make validate
```

It formats the project, runs Clippy with warnings denied, and runs all tests. Integration tests that use `#[sqlx::test]` require PostgreSQL and permission to create temporary test databases.

The individual commands are:

```bash
make format
make lint
make test
```

A successful test run ends with `test result: ok` and no failed tests.

## Docker

Run the production image with the local `.env` file:

```bash
make run-docker-local
```

This command rebuilds the image, maps local PostgreSQL and SMTP hosts to `host.docker.internal`, binds the API only to `127.0.0.1:3000`, drops Linux capabilities, prevents privilege escalation, and uses a read-only container filesystem.

Keep `localhost` in the local `DATABASE_URL` and `SMTP_URL` values so the Make target can perform that host mapping. Press `Ctrl+C` to trigger graceful shutdown.

Build without running:

```bash
make build-docker
```

## Observability and operations

- Use `/api/v1/health/live` to check that the process is running.
- Use `/api/v1/health/ready` to check that the application can reach PostgreSQL.
- Capture the `x-request-id` response header in frontend error reports.
- Search request logs for the matching `request_id` when investigating failures.
- Monitor HTTP status and latency from the request logs.
- A readiness response of `503` means the database check failed.
- Email failures point to `SMTP_URL` or `MAIL_FROM`; image failures point to the R2 credentials, bucket, or public base URL.

## Deployment checklist

- Inject environment variables through the deployment platform's secret manager; never copy `.env` into the image.
- Terminate HTTPS at the platform load balancer or reverse proxy.
- Expose container port `3000` only through the platform service.
- Configure liveness and readiness probes with the health endpoints above.
- Preserve `SIGINT` delivery so the HTTP server and cleanup worker can shut down gracefully.
- Run `make validate` and `make build-docker` before release.

## Project layout

- `src/main.rs`: application startup and graceful shutdown
- `src/lib.rs`: router and shared HTTP middleware
- `src/auth.rs`, `src/profile.rs`, `src/product.rs`: API features
- `src/error.rs`: shared client-safe error responses
- `src/mail.rs`, `src/storage.rs`: external service adapters
- `src/object_cleanup.rs`: retryable object deletion worker
- `migrations/`: versioned PostgreSQL schema
- `tests/`: router-level integration tests
