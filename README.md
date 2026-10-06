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

`GET /health` does not require the bearer token. Every other path, including `/mcp`, rejects a missing or wrong `Authorization` header with `401`.

## Tools

Amounts in tool arguments and results are **currency units** (euros, dollars, …). The server converts to and from YNAB milliunits (1 unit = 1000 milliunits). When YNAB sends `*_currency` or `*_formatted`, those values are preferred. Category objects also include `assigned` (same as `budgeted`) and `available` (same as YNAB's `balance`). Month objects include `ready_to_assign` (same as `to_be_budgeted`).

Outflows are negative (`-12.50`). Inflows are positive. A dot decimal (`-12.50`) or a comma decimal with 1–2 digits (`12,50`) is accepted. `12,500` is rejected so a thousands separator is not guessed.

`plan_id` may be omitted when `YNAB_PLAN_ID` is set. `last-used` and `default` are also valid plan ids.

| Tool | Access | What it does |
| --- | --- | --- |
| `list_plans` | read | Plans visible to the access token |
| `get_plan` | read | One plan, including related entities. Full export; prefer narrower list tools |
| `get_plan_settings` | read | Date format and currency format |
| `get_user` | read | User id for the access token. No `plan_id` |
| `list_accounts` | read | Accounts, balances, and `transfer_payee_id` |
| `create_account` | write | New account. `name`, `type`, and `balance` (currency units) |
| `list_categories` | read | Categories for a month, with assigned / activity / available |
| `get_category` | read | One category for the current month, including its goal |
| `get_month_summary` | read | Income, assigned, activity, Ready to Assign, and categories |
| `list_months` | read | Every plan month, with income, assigned, activity, and Ready to Assign |
| `list_transactions` | read | Transactions by date, account, category, payee, or text search |
| `get_transaction` | read | One transaction by id |
| `import_transactions` | write | Import linked-account transactions. No body; returns imported ids |
| `list_scheduled_transactions` | read | Upcoming and recurring scheduled transactions |
| `get_scheduled_transaction` | read | One scheduled transaction by id |
| `list_payees` | read | Payees, including transfer payees |
| `create_payee` | write | New payee. `name` is required and at most 500 characters |
| `update_payee` | write | Rename a payee |
| `create_transaction` | write | One transaction or several. Approved unless `approved` is false |
| `update_transaction` | write | Patch one transaction, or several by id |
| `delete_transaction` | write | Delete one transaction |
| `set_transaction_approval` | write | Approve or unapprove. YNAB exposes this as the `approved` flag |
| `create_category_group` | write | Create a category group. `name` is required and at most 50 characters |
| `update_category_group` | write | Rename a category group. `name` is required and at most 50 characters |
| `create_category` | write | Create a category in an existing group. Requires `name` and `category_group_id` |
| `update_category` | write | Change name, note, group, or goal. Does not change the assigned amount |
| `update_category_budget` | write | Set (`assigned`) or nudge (`adjust_by`) a category for a month |
| `move_money` | write | Move a positive amount between categories, or to/from Ready to Assign |
| `list_money_movements` | read | Movements between categories, or between a category and Ready to Assign |
| `create_scheduled_transaction` | write | Future or recurring transaction |
| `update_scheduled_transaction` | write | Update a scheduled transaction. Omitted fields stay as they are |
| `delete_scheduled_transaction` | write | Delete a scheduled transaction |

To transfer between accounts, call `list_accounts` and use the destination account's `transfer_payee_id` as `payee_id`.

To add an envelope, call `create_category_group` (or reuse a group id from `list_categories`), then `create_category`. YNAB's plans API accepts both creates: `POST /plans/{plan_id}/category_groups` with `{ "category_group": { "name" } }`, and `POST /plans/{plan_id}/categories` with `{ "category": { "name", "category_group_id" } }`. Optional category fields are `note`, `goal_target` (currency units; YNAB stores milliunits and creates a monthly goal), `goal_target_date` (`YYYY-MM-DD`), and `goal_needs_whole_amount`. Category names are limited to 200 characters, notes to 500, and group names to 50. YNAB rejects an internal group such as Credit Card Payments. The response `result` object includes the new category or group and `server_knowledge`, with money fields in currency units.

`update_category` sends `PATCH /plans/{plan_id}/categories/{category_id}` with only the fields you set: `name`, `note`, `category_group_id`, `goal_target`, `goal_target_date`, `goal_needs_whole_amount`, and `goal_frequency` (`monthly`, `weekly`, or `yearly`). Omitted fields stay as they are. An empty `note` clears the note. `goal_frequency` requires `goal_target` and cannot be combined with `goal_target_date`. It configures a recurring target and replaces the existing cadence. `update_category_group` sends `PATCH /plans/{plan_id}/category_groups/{category_group_id}` with `{ "category_group": { "name" } }`.

`create_account` sends `POST /plans/{plan_id}/accounts` with `name`, `type` (`checking`, `savings`, `cash`, `creditCard`, `otherAsset`, `otherLiability`), and `balance` in milliunits. `create_payee` and `update_payee` send `name` only (at most 500 characters). `import_transactions` is `POST /plans/{plan_id}/transactions/import` with no body. `list_money_movements` calls `GET /plans/{plan_id}/money_movements`, or `GET /plans/{plan_id}/months/{month}/money_movements` when `month` is set. `get_plan` is `GET /plans/{plan_id}` and `get_plan_settings` is `GET /plans/{plan_id}/settings`. `get_user` is `GET /user`.

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

Tests, including a mocked YNAB API (no real token):

```sh
cargo test
```

## Docker

The image listens on `0.0.0.0:8080`. Publish that port. Pass secrets as environment variables; do not bake them into the image.

```sh
docker build -t ynab-mcp .
docker run --rm -p 8080:8080 \
  -e YNAB_API_KEY \
  -e MCP_AUTH_TOKEN \
  -e YNAB_PLAN_ID \
  -e MCP_ALLOWED_HOSTS \
  ynab-mcp
```

`YNAB_API_KEY` and `MCP_AUTH_TOKEN` are required. `YNAB_PLAN_ID` is optional. `MCP_ALLOWED_HOSTS` is optional and defaults to `localhost,127.0.0.1,::1`. When the container is reached through another hostname, set it to that name, for example `your-mcp.example.com`. An entry without a port matches any port. Include the loopback names as well if checks from inside the container should keep working.

`docker compose up --build` reads `.env` and publishes port 8080.

```yaml
services:
  ynab-mcp:
    build: .
    ports:
      - "8080:8080"
    env_file:
      - .env
```

Health check: `GET /health` returns `200` and `{"status":"healthy","service":"ynab-mcp"}`. The image runs the same check. MCP is `POST /mcp` with `Authorization: Bearer <MCP_AUTH_TOKEN>`.

Point Cursor at `https://your-host.example.com/mcp`. Terminate TLS in front of the container if clients use HTTPS.

The rate-limit window and GET cache live in memory in this process.

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
