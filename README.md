# Arcanon Scanner

A Rust CLI that statically analyzes codebases to extract service boundaries, endpoints, connections, and schemas — then uploads the results to Arcanon Hub as a `ScanPayloadV1`. Runs locally on developer machines or in CI with zero cloud dependency and zero LLM requirement.

## Install

```bash
# From source (requires Rust 1.85+)
git clone https://github.com/arcanon-dev/arcanon-scanner.git
cd arcanon-scanner
make install

# Or directly via cargo
cargo install --path .
```

To uninstall:

```bash
make uninstall
```

## Usage

```
arcanon-scanner [OPTIONS] [PATH]
```

Scan the current directory and upload to hub:

```bash
arcanon-scanner --hub-url https://hub.arcanon.dev --api-key $ARCANON_API_KEY
```

Preview output without uploading:

```bash
arcanon-scanner --dry-run
```

Save payload to a file:

```bash
arcanon-scanner --output scan-result.json
```

Scan a specific directory:

```bash
arcanon-scanner /path/to/repo --dry-run
```

## CLI Options

| Option | Env Variable | Description |
|--------|-------------|-------------|
| `--hub-url <URL>` | `ARCANON_HUB_URL` | Hub API endpoint |
| `--api-key <KEY>` | `ARCANON_API_KEY` | API key for upload auth |
| `--project-slug <SLUG>` | `ARCANON_PROJECT_SLUG` | Project slug for multi-repo grouping |
| `--output <FILE>` | | Write payload JSON to file instead of uploading |
| `--dry-run` | | Print payload to stdout, don't upload |
| `--plugins <LIST>` | | Comma-separated plugin filter (e.g. `typescript,openapi`) |
| `--exclude <GLOB>` | | Additional exclude patterns (repeatable) |
| `--repo-url <URL>` | `ARCANON_REPO_URL` | Override git remote detection |
| `--branch <NAME>` | `ARCANON_BRANCH` | Override branch detection |
| `--commit-sha <SHA>` | `ARCANON_COMMIT_SHA` | Override commit SHA detection |
| `-v` / `-vv` / `-vvv` | | Increase log verbosity (info / debug / trace) |
| `--version` | | Print version and exit |

**Precedence:** CLI flags > environment variables > `.arcanon.toml` > built-in defaults.

## Configuration (.arcanon.toml)

Place an `.arcanon.toml` file in your repo root. This file is meant to be checked into version control.

```toml
[scanner]
project_slug = "acme-platform"         # default --project-slug
hub_url = "https://hub.arcanon.dev"    # default --hub-url (secrets stay in env vars)

[scanner.exclude]
# Glob patterns to exclude (in addition to built-in excludes)
paths = [
    "vendor/**",
    "legacy/**",
    "**/*.generated.ts",
]

[scanner.plugins]
# Explicitly enable/disable plugins (default: all enabled)
# disabled = ["ruby", "asyncapi"]

[services]
# Override or hint service names when auto-detection gets it wrong
# Key = directory path relative to repo root

[services."packages/api"]
name = "api-server"                    # override auto-detected name
language = "typescript"                # hint when ambiguous

[services."packages/worker"]
name = "background-worker"

[services.shared]
# Shared libraries are not services — exclude from service detection
ignore = true

[connections]
# Manual connection declarations for things the scanner can't detect
# (e.g., runtime-only service discovery, sidecar proxies)

[[connections.manual]]
source = "api-server"
target = "auth-proxy"
protocol = "rest"
path = "/auth/verify"
confidence = "high"
```

## What It Detects

### Languages (7 plugins)

| Language | Frameworks | HTTP Clients | MQ | DB | Other |
|----------|-----------|-------------|-----|-----|-------|
| **TypeScript** | Express, NestJS, Fastify, Next.js | fetch, axios, got, ky, superagent | KafkaJS, amqplib | mongoose, pg, Prisma, TypeORM, Sequelize, Redis | gRPC |
| **Python** | FastAPI, Django, Flask | requests, httpx, aiohttp | pika, Celery, NATS | asyncpg, psycopg2, SQLAlchemy, motor | Modbus, OPC UA, BACnet, gRPC |
| **Go** | Gin, Echo, Fiber, Chi, gorilla/mux, net/http | http.Get/Post | Kafka (sarama) | sql.Open, sqlx, MongoDB, Redis | gRPC, NATS |
| **Java** | Spring Boot | RestTemplate, WebClient, FeignClient | Kafka, RabbitMQ | JDBC, JPA | gRPC |
| **C#** | ASP.NET Core, Minimal API | HttpClient, IHttpClientFactory | MassTransit | EF Core | gRPC |
| **Rust** | Actix-web, Axum, Rocket | reqwest | | | tonic (gRPC), tokio-modbus |
| **Ruby** | Rails, Sinatra | Faraday, Net::HTTP, HTTParty | Sidekiq, ActiveJob | ActiveRecord | |

### Config Files (8 plugins)

| Plugin | Files | Extracts |
|--------|-------|----------|
| OpenAPI | `openapi.{json,yaml}`, `swagger.{json,yaml}` | Endpoints, schemas, service name |
| Proto | `*.proto` | gRPC services, rpc methods, message schemas |
| GraphQL | `*.graphql`, `*.gql` | Queries, mutations, subscriptions, types |
| AsyncAPI | `asyncapi.{json,yaml}` | Message channels, event schemas |
| Docker Compose | `docker-compose*.{yml,yaml}` | Services, depends_on, ports, env vars |
| Kubernetes | `k8s/**/*.{yml,yaml}` | Services, Deployments, ConfigMaps |
| Dockerfile | `Dockerfile*`, `Containerfile*` | Service boundaries |
| .env | `.env*` | Variable values for resolution chain |

## Built-in Excludes

These directories are always excluded and cannot be overridden:

`.git/`, `node_modules/`, `__pycache__/`, `.tox/`, `.mypy_cache/`, `.pytest_cache/`, `target/`, `dist/`, `build/`, `out/`, `.next/`, `vendor/`

Files are also skipped if they exceed 500KB, contain null bytes (binary), or have lines exceeding 10,000 characters (minified).

## CI Integration

```yaml
# GitHub Actions
- name: Scan with Arcanon
  run: |
    arcanon-scanner \
      --hub-url ${{ secrets.ARCANON_HUB_URL }} \
      --api-key ${{ secrets.ARCANON_API_KEY }} \
      --project-slug my-project
```

## Development

```bash
make lint      # cargo clippy -- -D warnings
make fmt       # cargo fmt --check
make test      # cargo test
make build     # cargo build + cargo build --release
make install   # cargo install --path .
make uninstall # cargo uninstall arcanon-scanner
```

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success (uploaded, saved, or dry-run) |
| 1 | Upload failed after retries |
| 2 | Invalid arguments |
