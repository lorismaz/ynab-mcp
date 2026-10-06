//! Smoke test: health, bearer auth, MCP initialize, tools/list, and a mocked YNAB write.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use ynab_mcp::{build_router, Config};

const MCP_TOKEN: &str = "test-mcp-auth-token-not-real";
const YNAB_TOKEN: &str = "test-ynab-api-key-not-real";

#[derive(Clone, Default)]
struct Hits {
    requests: Arc<Mutex<Vec<Recorded>>>,
}

struct Recorded {
    method: String,
    path: String,
    authorization: String,
    body: Value,
}

#[tokio::test]
async fn health_initialize_and_tools_over_http() {
    let hits = Hits::default();
    let ynab_base = spawn_ynab(hits.clone()).await;
    let config = Config::for_test(ynab_base, MCP_TOKEN, YNAB_TOKEN);
    let app = build_router(config).expect("router");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let root = format!("http://{address}");

    let health = client.get(format!("{root}/health")).send().await.unwrap();
    assert_eq!(health.status(), 200);
    let health_body = health.text().await.unwrap();
    assert!(health_body.contains("healthy"));
    assert!(!health_body.contains(MCP_TOKEN));
    assert!(!health_body.contains(YNAB_TOKEN));

    let denied = client
        .post(format!("{root}/mcp"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(initialize_body())
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    let denied_body = denied.text().await.unwrap();
    assert!(denied_body.contains("unauthorized"));
    assert!(!denied_body.contains(MCP_TOKEN));

    let wrong = client
        .post(format!("{root}/mcp"))
        .header("authorization", "Bearer definitely-not-the-token")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(initialize_body())
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);

    let initialized = mcp_call(
        &client,
        &root,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "ynab-mcp-smoke", "version": "0.1.0"}
            }
        }),
    )
    .await;
    assert_eq!(initialized["result"]["serverInfo"]["name"], "ynab");
    assert!(initialized["result"]["capabilities"]["tools"].is_object());

    let listed = mcp_call(
        &client,
        &root,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await;
    let names = tool_names(&listed);
    for required in [
        "list_plans",
        "list_accounts",
        "list_categories",
        "get_month_summary",
        "list_transactions",
        "list_scheduled_transactions",
        "list_payees",
        "create_transaction",
        "update_transaction",
        "delete_transaction",
        "set_transaction_approval",
        "create_category_group",
        "create_category",
        "update_category_budget",
        "move_money",
        "create_scheduled_transaction",
        "update_scheduled_transaction",
        "delete_scheduled_transaction",
    ] {
        assert!(
            names.iter().any(|name| name == required),
            "missing {required}"
        );
    }

    let plans = mcp_call(
        &client,
        &root,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "list_plans", "arguments": {}}
        }),
    )
    .await;
    let plans_text = plans["result"]["content"][0]["text"].as_str().unwrap();
    let plans_json: Value = serde_json::from_str(plans_text).unwrap();
    assert_eq!(plans_json["plans"][0]["name"], "Famille");
    assert!(!plans_text.contains(YNAB_TOKEN));

    let created = mcp_call(
        &client,
        &root,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "create_transaction",
                "arguments": {
                    "plan_id": "plan-1",
                    "transaction": {
                        "account_id": "acc-1",
                        "date": "2026-10-01",
                        "amount": -12.50,
                        "payee_name": "Boulangerie"
                    }
                }
            }
        }),
    )
    .await;
    assert_eq!(created["result"]["isError"], false);
    let created_text = created["result"]["content"][0]["text"].as_str().unwrap();
    let created_json: Value = serde_json::from_str(created_text).unwrap();
    assert_eq!(
        created_json["result"]["transaction"]["amount"],
        json!(-12.5)
    );

    let recorded = hits.requests.lock().unwrap();
    let create = recorded
        .iter()
        .find(|hit| hit.method == "POST" && hit.path.ends_with("/transactions"))
        .expect("create transaction was sent to YNAB");
    assert_eq!(create.authorization, format!("Bearer {YNAB_TOKEN}"));
    assert_eq!(create.body["transaction"]["amount"], json!(-12500));
    assert_eq!(create.body["transaction"]["approved"], json!(true));
}

#[tokio::test]
async fn create_category_and_group_over_http() {
    let hits = Hits::default();
    let ynab_base = spawn_ynab_categories(hits.clone()).await;
    let config = Config::for_test(ynab_base, MCP_TOKEN, YNAB_TOKEN);
    let app = build_router(config).expect("router");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let root = format!("http://{address}");

    let grouped = mcp_call(
        &client,
        &root,
        json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": "create_category_group",
                "arguments": {"plan_id": "plan-1", "name": "  Vacances  "}
            }
        }),
    )
    .await;
    assert_eq!(grouped["result"]["isError"], false);
    let grouped_text = grouped["result"]["content"][0]["text"].as_str().unwrap();
    let grouped_json: Value = serde_json::from_str(grouped_text).unwrap();
    assert_eq!(grouped_json["plan_id"], "plan-1");
    assert_eq!(grouped_json["result"]["category_group"]["id"], "group-1");
    assert_eq!(grouped_json["result"]["category_group"]["name"], "Vacances");

    let created = mcp_call(
        &client,
        &root,
        json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": {
                "name": "create_category",
                "arguments": {
                    "plan_id": "plan-1",
                    "category_group_id": "group-1",
                    "name": " Hotel ",
                    "note": "Mazafati",
                    "goal_target": 1500,
                    "goal_target_date": "2026-08-01",
                    "goal_needs_whole_amount": false
                }
            }
        }),
    )
    .await;
    assert_eq!(created["result"]["isError"], false);
    let created_text = created["result"]["content"][0]["text"].as_str().unwrap();
    let created_json: Value = serde_json::from_str(created_text).unwrap();
    assert_eq!(created_json["result"]["category"]["id"], "cat-1");
    assert_eq!(
        created_json["result"]["category"]["goal_target"],
        json!(1500.0)
    );
    assert_eq!(created_json["result"]["category"]["assigned"], json!(0.0));
    assert!(!created_text.contains(YNAB_TOKEN));

    let blank = mcp_call(
        &client,
        &root,
        json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "tools/call",
            "params": {
                "name": "create_category",
                "arguments": {
                    "plan_id": "plan-1",
                    "category_group_id": "group-1",
                    "name": "   "
                }
            }
        }),
    )
    .await;
    assert_eq!(blank["result"]["isError"], true);
    let blank_text = blank["result"]["content"][0]["text"].as_str().unwrap();
    assert!(blank_text.contains("name is required"), "{blank_text}");

    let long_group = mcp_call(
        &client,
        &root,
        json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": "tools/call",
            "params": {
                "name": "create_category_group",
                "arguments": {"plan_id": "plan-1", "name": "x".repeat(51)}
            }
        }),
    )
    .await;
    assert_eq!(long_group["result"]["isError"], true);
    let long_text = long_group["result"]["content"][0]["text"].as_str().unwrap();
    assert!(long_text.contains("at most 50"), "{long_text}");

    let negative_goal = mcp_call(
        &client,
        &root,
        json!({
            "jsonrpc": "2.0",
            "id": 14,
            "method": "tools/call",
            "params": {
                "name": "create_category",
                "arguments": {
                    "plan_id": "plan-1",
                    "category_group_id": "group-1",
                    "name": "Hotel",
                    "goal_target": -10
                }
            }
        }),
    )
    .await;
    assert_eq!(negative_goal["result"]["isError"], true);

    let recorded = hits.requests.lock().unwrap();
    assert_eq!(recorded.len(), 2);
    let group = &recorded[0];
    assert_eq!(group.method, "POST");
    assert_eq!(group.path, "/v1/plans/plan-1/category_groups");
    assert_eq!(group.authorization, format!("Bearer {YNAB_TOKEN}"));
    assert_eq!(group.body["category_group"]["name"], json!("Vacances"));
    let category = &recorded[1];
    assert_eq!(category.method, "POST");
    assert_eq!(category.path, "/v1/plans/plan-1/categories");
    assert_eq!(category.body["category"]["name"], json!("Hotel"));
    assert_eq!(
        category.body["category"]["category_group_id"],
        json!("group-1")
    );
    assert_eq!(category.body["category"]["note"], json!("Mazafati"));
    assert_eq!(category.body["category"]["goal_target"], json!(1_500_000));
    assert_eq!(
        category.body["category"]["goal_target_date"],
        json!("2026-08-01")
    );
    assert_eq!(
        category.body["category"]["goal_needs_whole_amount"],
        json!(false)
    );
}

async fn mcp_call(client: &reqwest::Client, root: &str, body: Value) -> Value {
    let response = client
        .post(format!("{root}/mcp"))
        .header("authorization", format!("Bearer {MCP_TOKEN}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.text().await.unwrap();
    assert!(
        status.is_success(),
        "MCP {} failed: {status} {bytes}",
        body["method"]
    );
    parse_mcp_body(&bytes)
}

fn parse_mcp_body(body: &str) -> Value {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return value;
    }
    let data = body
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
        .unwrap_or(body);
    serde_json::from_str(data).unwrap_or_else(|error| panic!("unparsed MCP body ({error}): {body}"))
}

fn tool_names(listed: &Value) -> Vec<String> {
    listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect()
}

fn initialize_body() -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "ynab-mcp-smoke", "version": "0.1.0"}
        }
    }))
    .unwrap()
}

async fn spawn_ynab(hits: Hits) -> String {
    let app = Router::new()
        .route("/v1/plans", get(mock_plans))
        .route(
            "/v1/plans/{plan_id}/transactions",
            post(mock_create_transaction),
        )
        .with_state(hits);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{port}/v1")
}

async fn mock_plans(State(hits): State<Hits>, headers: HeaderMap) -> Json<Value> {
    record(&hits, "GET", "/v1/plans", &headers, Value::Null);
    Json(json!({
        "data": {
            "plans": [{
                "id": "plan-1",
                "name": "Famille",
                "currency_format": {"iso_code": "EUR"}
            }]
        }
    }))
}

async fn mock_create_transaction(
    State(hits): State<Hits>,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    record(
        &hits,
        "POST",
        &format!("/v1/plans/{plan_id}/transactions"),
        &headers,
        body,
    );
    Json(json!({
        "data": {
            "transaction_ids": ["tx-1"],
            "server_knowledge": 1,
            "transaction": {
                "id": "tx-1",
                "date": "2026-10-01",
                "amount": -12500,
                "amount_currency": -12.5,
                "amount_formatted": "-12,50 €",
                "approved": true,
                "cleared": "uncleared",
                "deleted": false,
                "account_id": "acc-1",
                "account_name": "Checking",
                "payee_name": "Boulangerie",
                "subtransactions": []
            }
        }
    }))
}

async fn spawn_ynab_categories(hits: Hits) -> String {
    let app = Router::new()
        .route(
            "/v1/plans/{plan_id}/category_groups",
            post(mock_create_category_group),
        )
        .route("/v1/plans/{plan_id}/categories", post(mock_create_category))
        .with_state(hits);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{port}/v1")
}

async fn mock_create_category_group(
    State(hits): State<Hits>,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    record(
        &hits,
        "POST",
        &format!("/v1/plans/{plan_id}/category_groups"),
        &headers,
        body.clone(),
    );
    let name = body["category_group"]["name"].as_str().unwrap_or("");
    Json(json!({
        "data": {
            "server_knowledge": 2,
            "category_group": {
                "id": "group-1",
                "name": name,
                "hidden": false,
                "deleted": false
            }
        }
    }))
}

async fn mock_create_category(
    State(hits): State<Hits>,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    record(
        &hits,
        "POST",
        &format!("/v1/plans/{plan_id}/categories"),
        &headers,
        body,
    );
    Json(json!({
        "data": {
            "server_knowledge": 3,
            "category": {
                "id": "cat-1",
                "category_group_id": "group-1",
                "category_group_name": "Vacances",
                "name": "Hotel",
                "hidden": false,
                "deleted": false,
                "note": "Mazafati",
                "budgeted": 0,
                "activity": 0,
                "balance": 0,
                "goal_target": 1500000,
                "goal_target_currency": 1500.0
            }
        }
    }))
}

fn record(hits: &Hits, method: &str, path: &str, headers: &HeaderMap, body: Value) {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    hits.requests.lock().unwrap().push(Recorded {
        method: method.into(),
        path: path.into(),
        authorization,
        body,
    });
}
