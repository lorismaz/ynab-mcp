# ynab-mcp

Self-hosted [Model Context Protocol](https://modelcontextprotocol.io) server for [YNAB](https://www.ynab.com/) (You Need A Budget). It speaks **streamable HTTP** and can create, update, and delete transactions on the plan behind a Personal Access Token.

Public URL (example): `https://your-host.example.com/mcp`.

## Why Rust

The official Rust SDK [`rmcp`](https://crates.io/crates/rmcp) 3.5 implements the current streamable HTTP transport (Axum + Tower), including JSON responses and the stateless mode used by protocol `2026-07-28` and by older clients when sessions are turned off. That is enough for a small production server, so this repo is Rust rather than a Node wrapper around `@modelcontextprotocol/sdk`.

The HTTP service is **stateless**: each `POST /mcp` is a complete JSON-RPC message. Clients do not need `Mcp-Session-Id`. Simple responses are `application/json`. Clients must still send `Accept: application/json, text/event-stream`, which the MCP streamable HTTP client does.

## Cursor

In Cursor, add an MCP server with URL `https://your-host.example.com/mcp` and a bearer header. The same block works in an `mcp.json`:

```json
{
  "mcpServers": {
    "ynab": {
      "url": "https://your-host.example.com/mcp",
      "headers": {
        "Authorization": "Bearer <MCP_AUTH_TOKEN>"
      }
    }
  }
}
```

Replace `<MCP_AUTH_TOKEN>` with the same secret configured on the server. Anyone who has that secret can read and change the YNAB plan. Treat it like a password.

`GET /health` does not require the bearer token, so a platform health check can use it. Every other path, including `/mcp`, rejects a missing or wrong `Authorization` header with `401`.

## Tools

Amounts in tool arguments and results are **currency units** (euros, dollars, …). The server converts to and from YNAB milliunits (1 unit = 1000 milliunits). When YNAB sends `*_currency` or `*_formatted`, those values are preferred. Category objects also include `assigned` (same as `budgeted`) and `available` (same as YNAB's `balance`). Month objects include `ready_to_assign` (same as `to_be_budgeted`).

Outflows are negative (`-12.50`). Inflows are positive. A dot decimal (`-12.50`) or a comma decimal with 1–2 digits (`12,50`) is accepted. `12,500` is rejected so a thousands separator is not guessed.

`plan_id` may be omitted when `YNAB_PLAN_ID` is set. `last-used` and `default` are also valid plan ids.

| Tool | Access | What it does |
| --- | --- | --- |
| `list_plans` | read | Plans visible to the access token |
| `list_accounts` | read | Accounts, balances, and `transfer_payee_id` |
| `list_categories` | read | Categories for a month, with assigned / activity / available |
| `get_month_summary` | read | Income, assigned, activity, Ready to Assign, and categories |
| `list_transactions` | read | Transactions by date, account, category, payee, or text search |
| `list_scheduled_transactions` | read | Upcoming and recurring scheduled transactions |
| `list_payees` | read | Payees, including transfer payees |
| `create_transaction` | write | One transaction or several. Approved unless `approved` is false |
| `update_transaction` | write | Patch one transaction, or several by id |
| `delete_transaction` | write | Delete one transaction |
| `set_transaction_approval` | write | Approve or unapprove. YNAB exposes this as the `approved` flag |
| `update_category_budget` | write | Set (`assigned`) or nudge (`adjust_by`) a category for a month |
| `move_money` | write | Move a positive amount between categories, or to/from Ready to Assign |
| `create_scheduled_transaction` | write | Future or recurring transaction |
| `update_scheduled_transaction` | write | Update a scheduled transaction. Omitted fields stay as they are |
| `delete_scheduled_transaction` | write | Delete a scheduled transaction |

To transfer between accounts, call `list_accounts` and use the destination account's `transfer_payee_id` as `payee_id`.

Months accept `YYYY-MM`, `YYYY-MM-DD` (normalized to the first of that month), or `current`.

YNAB allows about **200 requests per hour**. GET responses are cached for `YNAB_CACHE_TTL_SECONDS` (default 30). A local cap of `YNAB_MAX_REQUESTS_PER_HOUR` (default 180) fails before the upstream limit. Writes clear the cache. `list_transactions` can pass `since_server_knowledge` from an earlier `server_knowledge` to ask YNAB for a delta.

## Environment

Copy `.env.example` to `.env`. Do not commit `.env`.

| Variable | Required | Purpose |
| --- | --- | --- |
| `YNAB_API_KEY` | yes | YNAB Personal Access Token. `YNAB_ACCESS_TOKEN` is an alias. Used only to call `https://api.ynab.com/v1` |
| `MCP_AUTH_TOKEN` | yes | Bearer secret for MCP clients. `MCP_BEARER_TOKEN` is an alias. At least 16 characters |
| `YNAB_PLAN_ID` | no | Default plan when a tool omits `plan_id` |
| `PORT` | no | Listen port. Default `8080`. Binds `0.0.0.0` |
| `YNAB_API_BASE` | no | Default `https://api.ynab.com/v1` |
| `YNAB_CACHE_TTL_SECONDS` | no | GET cache TTL. `0` disables it. Default `30` |
| `YNAB_MAX_REQUESTS_PER_HOUR` | no | Local cap. `0` disables it. Default `180` |
| `MCP_ALLOWED_HOSTS` | no | Comma-separated `Host` values allowed to call `/mcp`. Default `localhost,127.0.0.1,::1`. Set this to your public hostname in production, for example `your-mcp.example.com`. An entry without a port matches any port |
| `RUST_LOG` | no | Tracing filter. Default `info`. Tokens are not logged |

Create a YNAB token under [Account Settings → Developer Settings](https://app.ynab.com/settings/developer). The token is stored only in the server environment. This process never prints it.

Generate the MCP secret with:

```sh
openssl rand -hex 32
```

## Local run

```sh
cp .env.example .env
# edit .env — real tokens stay out of git
cargo run --release
```

```sh
curl -sS http://127.0.0.1:8080/health
```

Docker Compose reads `.env` and publishes port 8080:

```sh
docker compose up --build
```

Tests, including a mocked YNAB API (no real token):

```sh
cargo test
```

## Deploy on Dokploy

1. In Dokploy, create an application from this GitHub repository (`lorismaz/ynab-mcp`). Build with the repository `Dockerfile` (multi-stage Rust build, Debian slim runtime, non-root user).
2. Set the container port to **8080**. The process listens on `0.0.0.0:$PORT`.
3. Attach your domain (for example **your-mcp.example.com**) and enable HTTPS (Let's Encrypt through Dokploy).
4. Set environment variables:
   - `YNAB_API_KEY` — the Personal Access Token
   - `MCP_AUTH_TOKEN` — the shared bearer secret
   - `YNAB_PLAN_ID` — optional
   - `MCP_ALLOWED_HOSTS` — the public hostname, for example `your-mcp.example.com` (the default is loopback only)
5. Health check: HTTP `GET /health` expecting `200`. The image also defines a Docker `HEALTHCHECK` against that path.
6. Deploy. Cursor should use `https://your-host.example.com/mcp` with `Authorization: Bearer <MCP_AUTH_TOKEN>`.

The streamable HTTP handler checks the `Host` header. The default allow list is loopback only (`localhost`, `127.0.0.1`, `::1`). Set `MCP_ALLOWED_HOSTS` to the hostname the proxy sends, for example `your-mcp.example.com`. Include loopback as well if local checks should keep working.

One replica is enough. The server keeps its rate-limit window and GET cache in memory.

## HTTP surface

| Method | Path | Auth | Body |
| --- | --- | --- | --- |
| `GET` | `/health` | no | `{"status":"healthy","service":"ynab-mcp"}` |
| `GET` | `/` | no | Service name and the `/mcp` path |
| `POST` | `/mcp` | bearer | One JSON-RPC message (`initialize`, `tools/list`, `tools/call`, …) |

`POST /mcp` requires:

```http
Authorization: Bearer <MCP_AUTH_TOKEN>
Content-Type: application/json
Accept: application/json, text/event-stream
```
