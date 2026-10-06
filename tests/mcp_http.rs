//! Smoke test: health, bearer auth, MCP initialize, tools/list, and a mocked YNAB write.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, patch, post},
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
        "create_category",
        "create_category_group",
        "update_category",
        "update_category_group",
        "create_account",
        "create_payee",
        "update_payee",
        "import_transactions",
        "list_months",
        "get_category",
        "get_user",
        "get_plan",
        "get_plan_settings",
        "list_money_movements",
        "get_transaction",
        "get_scheduled_transaction",
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

#[tokio::test]
async fn plans_reads_and_writes_over_http() {
    let hits = Hits::default();
    let ynab_base = spawn_ynab_plans(hits.clone()).await;
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

    let missing_field = call_tool(
        &client,
        &root,
        20,
        "update_category",
        json!({"plan_id": "plan-1", "category_id": "cat-1"}),
    )
    .await;
    assert_eq!(missing_field["result"]["isError"], true);
    let missing_text = tool_text(&missing_field);
    assert!(
        missing_text.contains("at least one field"),
        "{missing_text}"
    );

    let frequency_alone = call_tool(
        &client,
        &root,
        21,
        "update_category",
        json!({
            "plan_id": "plan-1",
            "category_id": "cat-1",
            "goal_frequency": "weekly"
        }),
    )
    .await;
    assert_eq!(frequency_alone["result"]["isError"], true);
    let frequency_text = tool_text(&frequency_alone);
    assert!(
        frequency_text.contains("goal_frequency requires goal_target"),
        "{frequency_text}"
    );

    let frequency_and_date = call_tool(
        &client,
        &root,
        22,
        "update_category",
        json!({
            "plan_id": "plan-1",
            "category_id": "cat-1",
            "goal_target": 10,
            "goal_target_date": "2026-12-01",
            "goal_frequency": "monthly"
        }),
    )
    .await;
    assert_eq!(frequency_and_date["result"]["isError"], true);
    let both_text = tool_text(&frequency_and_date);
    assert!(both_text.contains("cannot be combined"), "{both_text}");

    assert!(hits.requests.lock().unwrap().is_empty());

    let updated = call_tool(
        &client,
        &root,
        23,
        "update_category",
        json!({
            "plan_id": "plan-1",
            "category_id": "cat-1",
            "name": " Hotel ",
            "note": "Mazafati",
            "category_group_id": "group-2",
            "goal_target": "20,50",
            "goal_needs_whole_amount": true,
            "goal_frequency": "weekly"
        }),
    )
    .await;
    assert_eq!(updated["result"]["isError"], false);
    let updated_json = tool_json(&updated);
    assert_eq!(updated_json["plan_id"], "plan-1");
    assert_eq!(
        updated_json["result"]["category"]["goal_target"],
        json!(20.5)
    );
    assert_eq!(updated_json["result"]["category"]["assigned"], json!(0.0));

    let cleared = call_tool(
        &client,
        &root,
        24,
        "update_category",
        json!({"plan_id": "plan-1", "category_id": "cat-1", "note": ""}),
    )
    .await;
    assert_eq!(cleared["result"]["isError"], false);

    let group = call_tool(
        &client,
        &root,
        25,
        "update_category_group",
        json!({"plan_id": "plan-1", "category_group_id": "group-1", "name": "  Trips  "}),
    )
    .await;
    assert_eq!(group["result"]["isError"], false);
    assert_eq!(
        tool_json(&group)["result"]["category_group"]["name"],
        "Trips"
    );

    let account = call_tool(
        &client,
        &root,
        26,
        "create_account",
        json!({
            "plan_id": "plan-1",
            "name": " Carte ",
            "type": "creditCard",
            "balance": -40.25
        }),
    )
    .await;
    assert_eq!(account["result"]["isError"], false);
    assert_eq!(
        tool_json(&account)["result"]["account"]["balance"],
        json!(-40.25)
    );

    let payee = call_tool(
        &client,
        &root,
        27,
        "create_payee",
        json!({"plan_id": "plan-1", "name": " Boulangerie "}),
    )
    .await;
    assert_eq!(payee["result"]["isError"], false);
    assert_eq!(tool_json(&payee)["result"]["payee"]["name"], "Boulangerie");

    let renamed = call_tool(
        &client,
        &root,
        28,
        "update_payee",
        json!({"plan_id": "plan-1", "payee_id": "payee-1", "name": "Baker"}),
    )
    .await;
    assert_eq!(renamed["result"]["isError"], false);

    let imported = call_tool(
        &client,
        &root,
        29,
        "import_transactions",
        json!({"plan_id": "plan-1"}),
    )
    .await;
    assert_eq!(imported["result"]["isError"], false);
    assert_eq!(
        tool_json(&imported)["result"]["transaction_ids"][0],
        "tx-imported"
    );

    let months = call_tool(
        &client,
        &root,
        30,
        "list_months",
        json!({"plan_id": "plan-1"}),
    )
    .await;
    assert_eq!(months["result"]["isError"], false);
    let months_json = tool_json(&months);
    assert_eq!(months_json["months"][0]["income"], json!(25.0));
    assert_eq!(months_json["months"][0]["ready_to_assign"], json!(10.0));
    assert_eq!(months_json["months"].as_array().unwrap().len(), 1);

    let category = call_tool(
        &client,
        &root,
        31,
        "get_category",
        json!({"plan_id": "plan-1", "category_id": "cat-1"}),
    )
    .await;
    assert_eq!(category["result"]["isError"], false);
    assert_eq!(tool_json(&category)["category"]["available"], json!(12.5));

    let user = call_tool(&client, &root, 32, "get_user", json!({})).await;
    assert_eq!(user["result"]["isError"], false);
    assert_eq!(tool_json(&user)["user"]["id"], "user-1");

    let plan = call_tool(
        &client,
        &root,
        33,
        "get_plan",
        json!({"plan_id": "plan-1", "since_server_knowledge": 4}),
    )
    .await;
    assert_eq!(plan["result"]["isError"], false);
    let plan_json = tool_json(&plan);
    assert_eq!(plan_json["plan"]["name"], "Famille");
    assert_eq!(plan_json["plan"]["accounts"][0]["balance"], json!(100.0));
    assert_eq!(plan_json["server_knowledge"], 9);

    let settings = call_tool(
        &client,
        &root,
        34,
        "get_plan_settings",
        json!({"plan_id": "plan-1"}),
    )
    .await;
    assert_eq!(settings["result"]["isError"], false);
    assert_eq!(
        tool_json(&settings)["settings"]["currency_format"]["iso_code"],
        "EUR"
    );

    let movements = call_tool(
        &client,
        &root,
        35,
        "list_money_movements",
        json!({"plan_id": "plan-1"}),
    )
    .await;
    assert_eq!(movements["result"]["isError"], false);
    let movements_json = tool_json(&movements);
    assert_eq!(movements_json["month"], Value::Null);
    assert_eq!(movements_json["money_movements"][0]["amount"], json!(5.0));

    let month_movements = call_tool(
        &client,
        &root,
        36,
        "list_money_movements",
        json!({"plan_id": "plan-1", "month": "2026-10"}),
    )
    .await;
    assert_eq!(month_movements["result"]["isError"], false);
    assert_eq!(tool_json(&month_movements)["month"], "2026-10-01");

    let transaction = call_tool(
        &client,
        &root,
        37,
        "get_transaction",
        json!({"plan_id": "plan-1", "transaction_id": "tx-1"}),
    )
    .await;
    assert_eq!(transaction["result"]["isError"], false);
    assert_eq!(
        tool_json(&transaction)["transaction"]["amount"],
        json!(-12.5)
    );

    let scheduled = call_tool(
        &client,
        &root,
        38,
        "get_scheduled_transaction",
        json!({"plan_id": "plan-1", "scheduled_transaction_id": "sched-1"}),
    )
    .await;
    assert_eq!(scheduled["result"]["isError"], false);
    assert_eq!(
        tool_json(&scheduled)["scheduled_transaction"]["amount"],
        json!(-30.0)
    );
    assert!(!tool_text(&scheduled).contains(YNAB_TOKEN));

    let recorded = hits.requests.lock().unwrap();
    assert_eq!(recorded.len(), 16);
    assert_eq!(recorded[0].method, "PATCH");
    assert_eq!(recorded[0].path, "/v1/plans/plan-1/categories/cat-1");
    assert_eq!(recorded[0].authorization, format!("Bearer {YNAB_TOKEN}"));
    assert_eq!(recorded[0].body["category"]["name"], json!("Hotel"));
    assert_eq!(recorded[0].body["category"]["note"], json!("Mazafati"));
    assert_eq!(
        recorded[0].body["category"]["category_group_id"],
        json!("group-2")
    );
    assert_eq!(recorded[0].body["category"]["goal_target"], json!(20_500));
    assert_eq!(
        recorded[0].body["category"]["goal_needs_whole_amount"],
        json!(true)
    );
    assert_eq!(
        recorded[0].body["category"]["goal_frequency"],
        json!("weekly")
    );
    assert!(recorded[0].body["category"]
        .get("goal_target_date")
        .is_none());
    assert_eq!(recorded[1].body["category"]["note"], Value::Null);
    assert_eq!(recorded[1].body["category"].as_object().unwrap().len(), 1);
    assert_eq!(recorded[2].method, "PATCH");
    assert_eq!(recorded[2].path, "/v1/plans/plan-1/category_groups/group-1");
    assert_eq!(recorded[2].body["category_group"]["name"], json!("Trips"));
    assert_eq!(recorded[3].method, "POST");
    assert_eq!(recorded[3].path, "/v1/plans/plan-1/accounts");
    assert_eq!(recorded[3].body["account"]["name"], json!("Carte"));
    assert_eq!(recorded[3].body["account"]["type"], json!("creditCard"));
    assert_eq!(recorded[3].body["account"]["balance"], json!(-40_250));
    assert_eq!(recorded[4].path, "/v1/plans/plan-1/payees");
    assert_eq!(recorded[4].body["payee"]["name"], json!("Boulangerie"));
    assert_eq!(recorded[5].method, "PATCH");
    assert_eq!(recorded[5].path, "/v1/plans/plan-1/payees/payee-1");
    assert_eq!(recorded[5].body["payee"]["name"], json!("Baker"));
    assert_eq!(recorded[6].method, "POST");
    assert_eq!(recorded[6].path, "/v1/plans/plan-1/transactions/import");
    assert_eq!(recorded[6].body, Value::Null);
    assert_eq!(recorded[7].method, "GET");
    assert_eq!(recorded[7].path, "/v1/plans/plan-1/months");
    assert_eq!(recorded[8].path, "/v1/plans/plan-1/categories/cat-1");
    assert_eq!(recorded[9].path, "/v1/user");
    assert_eq!(
        recorded[10].path,
        "/v1/plans/plan-1?last_knowledge_of_server=4"
    );
    assert_eq!(recorded[11].path, "/v1/plans/plan-1/settings");
    assert_eq!(recorded[12].path, "/v1/plans/plan-1/money_movements");
    assert_eq!(
        recorded[13].path,
        "/v1/plans/plan-1/months/2026-10-01/money_movements"
    );
    assert_eq!(recorded[14].path, "/v1/plans/plan-1/transactions/tx-1");
    assert_eq!(
        recorded[15].path,
        "/v1/plans/plan-1/scheduled_transactions/sched-1"
    );
}

async fn call_tool(
    client: &reqwest::Client,
    root: &str,
    id: i64,
    name: &str,
    arguments: Value,
) -> Value {
    mcp_call(
        client,
        root,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }),
    )
    .await
}

fn tool_text(response: &Value) -> String {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn tool_json(response: &Value) -> Value {
    serde_json::from_str(tool_text(response).as_str()).unwrap()
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

async fn spawn_ynab_plans(hits: Hits) -> String {
    let app = Router::new()
        .route(
            "/v1/plans/{plan_id}/categories/{category_id}",
            patch(mock_update_category).get(mock_get_category),
        )
        .route(
            "/v1/plans/{plan_id}/category_groups/{category_group_id}",
            patch(mock_update_category_group),
        )
        .route("/v1/plans/{plan_id}/accounts", post(mock_create_account))
        .route(
            "/v1/plans/{plan_id}/payees/{payee_id}",
            patch(mock_update_payee),
        )
        .route("/v1/plans/{plan_id}/payees", post(mock_create_payee))
        .route(
            "/v1/plans/{plan_id}/transactions/import",
            post(mock_import_transactions),
        )
        .route("/v1/plans/{plan_id}/months", get(mock_list_months))
        .route(
            "/v1/plans/{plan_id}/months/{month}/money_movements",
            get(mock_month_movements),
        )
        .route(
            "/v1/plans/{plan_id}/money_movements",
            get(mock_money_movements),
        )
        .route("/v1/user", get(mock_user))
        .route("/v1/plans/{plan_id}/settings", get(mock_plan_settings))
        .route("/v1/plans/{plan_id}", get(mock_plan))
        .route(
            "/v1/plans/{plan_id}/transactions/{transaction_id}",
            get(mock_transaction),
        )
        .route(
            "/v1/plans/{plan_id}/scheduled_transactions/{scheduled_transaction_id}",
            get(mock_scheduled),
        )
        .with_state(hits);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{port}/v1")
}

async fn mock_update_category(
    State(hits): State<Hits>,
    Path((plan_id, category_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    record(
        &hits,
        "PATCH",
        &format!("/v1/plans/{plan_id}/categories/{category_id}"),
        &headers,
        body,
    );
    Json(json!({
        "data": {
            "server_knowledge": 5,
            "category": {
                "id": "cat-1",
                "name": "Hotel",
                "budgeted": 0,
                "activity": 0,
                "balance": 0,
                "goal_target": 20500,
                "goal_target_currency": 20.5
            }
        }
    }))
}

async fn mock_get_category(
    State(hits): State<Hits>,
    Path((plan_id, category_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Json<Value> {
    record(
        &hits,
        "GET",
        &format!("/v1/plans/{plan_id}/categories/{category_id}"),
        &headers,
        Value::Null,
    );
    Json(json!({
        "data": {
            "category": {
                "id": category_id,
                "name": "Hotel",
                "budgeted": 10000,
                "activity": 2500,
                "balance": 12500,
                "balance_currency": 12.5,
                "budgeted_currency": 10.0,
                "activity_currency": 2.5
            }
        }
    }))
}

async fn mock_update_category_group(
    State(hits): State<Hits>,
    Path((plan_id, category_group_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    record(
        &hits,
        "PATCH",
        &format!("/v1/plans/{plan_id}/category_groups/{category_group_id}"),
        &headers,
        body.clone(),
    );
    Json(json!({
        "data": {
            "category_group": {
                "id": category_group_id,
                "name": body["category_group"]["name"],
                "hidden": false,
                "deleted": false
            }
        }
    }))
}

async fn mock_create_account(
    State(hits): State<Hits>,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    record(
        &hits,
        "POST",
        &format!("/v1/plans/{plan_id}/accounts"),
        &headers,
        body.clone(),
    );
    Json(json!({
        "data": {
            "account": {
                "id": "acc-new",
                "name": body["account"]["name"],
                "type": body["account"]["type"],
                "balance": body["account"]["balance"],
                "balance_currency": -40.25,
                "cleared_balance": 0,
                "uncleared_balance": 0,
                "closed": false,
                "deleted": false,
                "on_budget": true
            }
        }
    }))
}

async fn mock_create_payee(
    State(hits): State<Hits>,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    record(
        &hits,
        "POST",
        &format!("/v1/plans/{plan_id}/payees"),
        &headers,
        body.clone(),
    );
    Json(json!({
        "data": {
            "payee": {
                "id": "payee-1",
                "name": body["payee"]["name"],
                "deleted": false
            }
        }
    }))
}

async fn mock_update_payee(
    State(hits): State<Hits>,
    Path((plan_id, payee_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    record(
        &hits,
        "PATCH",
        &format!("/v1/plans/{plan_id}/payees/{payee_id}"),
        &headers,
        body,
    );
    Json(json!({
        "data": {
            "payee": {"id": payee_id, "name": "Baker", "deleted": false}
        }
    }))
}

async fn mock_import_transactions(
    State(hits): State<Hits>,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
) -> Json<Value> {
    record(
        &hits,
        "POST",
        &format!("/v1/plans/{plan_id}/transactions/import"),
        &headers,
        Value::Null,
    );
    Json(json!({"data": {"transaction_ids": ["tx-imported"]}}))
}

async fn mock_list_months(
    State(hits): State<Hits>,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
) -> Json<Value> {
    record(
        &hits,
        "GET",
        &format!("/v1/plans/{plan_id}/months"),
        &headers,
        Value::Null,
    );
    Json(json!({
        "data": {
            "server_knowledge": 6,
            "months": [
                {
                    "month": "2026-10-01",
                    "income": 25000,
                    "income_currency": 25.0,
                    "budgeted": 15000,
                    "budgeted_currency": 15.0,
                    "activity": -5000,
                    "activity_currency": -5.0,
                    "to_be_budgeted": 10000,
                    "to_be_budgeted_currency": 10.0,
                    "deleted": false
                },
                {
                    "month": "2026-09-01",
                    "income": 0,
                    "budgeted": 0,
                    "activity": 0,
                    "to_be_budgeted": 0,
                    "deleted": true
                }
            ]
        }
    }))
}

async fn mock_money_movements(
    State(hits): State<Hits>,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
) -> Json<Value> {
    record(
        &hits,
        "GET",
        &format!("/v1/plans/{plan_id}/money_movements"),
        &headers,
        Value::Null,
    );
    movement_body()
}

async fn mock_month_movements(
    State(hits): State<Hits>,
    Path((plan_id, month)): Path<(String, String)>,
    headers: HeaderMap,
) -> Json<Value> {
    record(
        &hits,
        "GET",
        &format!("/v1/plans/{plan_id}/months/{month}/money_movements"),
        &headers,
        Value::Null,
    );
    movement_body()
}

fn movement_body() -> Json<Value> {
    Json(json!({
        "data": {
            "server_knowledge": 7,
            "money_movements": [{
                "id": "move-1",
                "month": "2026-10-01",
                "amount": 5000,
                "from_category_id": "cat-a",
                "to_category_id": "cat-b"
            }]
        }
    }))
}

async fn mock_user(State(hits): State<Hits>, headers: HeaderMap) -> Json<Value> {
    record(&hits, "GET", "/v1/user", &headers, Value::Null);
    Json(json!({"data": {"user": {"id": "user-1"}}}))
}

async fn mock_plan(
    State(hits): State<Hits>,
    Path(plan_id): Path<String>,
    Query(query): Query<Vec<(String, String)>>,
    headers: HeaderMap,
) -> Json<Value> {
    let mut path = format!("/v1/plans/{plan_id}");
    if !query.is_empty() {
        let encoded = query
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&");
        path = format!("{path}?{encoded}");
    }
    record(&hits, "GET", &path, &headers, Value::Null);
    Json(json!({
        "data": {
            "server_knowledge": 9,
            "plan": {
                "id": plan_id,
                "name": "Famille",
                "accounts": [{
                    "id": "acc-1",
                    "name": "Checking",
                    "balance": 100000
                }]
            }
        }
    }))
}

async fn mock_plan_settings(
    State(hits): State<Hits>,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
) -> Json<Value> {
    record(
        &hits,
        "GET",
        &format!("/v1/plans/{plan_id}/settings"),
        &headers,
        Value::Null,
    );
    Json(json!({
        "data": {
            "settings": {
                "date_format": {"format": "DD/MM/YYYY"},
                "currency_format": {"iso_code": "EUR", "currency_symbol": "€"}
            }
        }
    }))
}

async fn mock_transaction(
    State(hits): State<Hits>,
    Path((plan_id, transaction_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Json<Value> {
    record(
        &hits,
        "GET",
        &format!("/v1/plans/{plan_id}/transactions/{transaction_id}"),
        &headers,
        Value::Null,
    );
    Json(json!({
        "data": {
            "server_knowledge": 8,
            "transaction": {
                "id": transaction_id,
                "amount": -12500,
                "date": "2026-10-01",
                "account_id": "acc-1"
            }
        }
    }))
}

async fn mock_scheduled(
    State(hits): State<Hits>,
    Path((plan_id, scheduled_transaction_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Json<Value> {
    record(
        &hits,
        "GET",
        &format!("/v1/plans/{plan_id}/scheduled_transactions/{scheduled_transaction_id}"),
        &headers,
        Value::Null,
    );
    Json(json!({
        "data": {
            "server_knowledge": 8,
            "scheduled_transaction": {
                "id": scheduled_transaction_id,
                "amount": -30000,
                "date_next": "2026-11-01",
                "frequency": "monthly"
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
