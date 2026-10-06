//! MCP tools. Amounts crossing this boundary are currency units.

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_handler, tool_router, ErrorData, ServerHandler,
};
use serde_json::{json, Map, Value};

use crate::{
    args::{
        ApprovalArgs, CreateAccountArgs, CreateCategoryArgs, CreateCategoryGroupArgs,
        CreatePayeeArgs, CreateScheduledArgs, CreateTransactionArgs, DeleteScheduledArgs,
        DeleteTransactionArgs, GetCategoryArgs, GetPlanArgs, GetScheduledTransactionArgs,
        GetTransactionArgs, GetUserArgs, ListAccountsArgs, ListCategoriesArgs,
        ListMoneyMovementsArgs, ListPayeesArgs, ListPlansArgs, ListTransactionsArgs,
        MonthSummaryArgs, MoveMoneyArgs, PlanArgs, ScheduledInput, SubtransactionInput,
        TransactionInput, TransactionPatch, UpdateCategoryArgs, UpdateCategoryBudgetArgs,
        UpdateCategoryGroupArgs, UpdatePayeeArgs, UpdateScheduledArgs, UpdateTransactionArgs,
    },
    money::milliunits_to_currency,
    present::present_money,
    validate::{check_len, normalize_date, normalize_month, require_future_date, validate_path_id},
    ynab::{CacheMode, YnabClient},
};

#[derive(Clone)]
pub struct YnabServer {
    client: YnabClient,
}

impl YnabServer {
    pub fn new(client: YnabClient) -> Self {
        Self { client }
    }

    fn plan_id(&self, explicit: Option<String>) -> Result<String, String> {
        let id = explicit
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| self.client.default_plan().map(str::to_string))
            .ok_or_else(|| {
                "plan_id is required. Call list_plans, pass plan_id, or set YNAB_PLAN_ID."
                    .to_string()
            })?;
        validate_path_id("plan_id", &id)?;
        Ok(id)
    }

    async fn list_plans_inner(&self, args: ListPlansArgs) -> Result<Value, String> {
        let mut query: Vec<(&str, &str)> = Vec::new();
        if args.include_accounts {
            query.push(("include_accounts", "true"));
        }
        let mut body = self
            .client
            .get("plans", &query, CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut body);
        let plans = body
            .pointer("/data/plans")
            .cloned()
            .unwrap_or_else(|| json!([]));
        Ok(json!({
            "plans": plans,
            "default_plan": body.pointer("/data/default_plan").cloned(),
            "configured_plan_id": self.client.default_plan(),
        }))
    }

    async fn list_accounts_inner(&self, args: ListAccountsArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let path = format!("plans/{plan_id}/accounts");
        let body = self
            .client
            .get_optional(&path, &[], CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        let (mut accounts, server_knowledge) = match body {
            None => (Vec::new(), None),
            Some(mut body) => {
                let knowledge = body.pointer("/data/server_knowledge").cloned();
                present_money(&mut body);
                let accounts = body
                    .pointer("/data/accounts")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                (accounts, knowledge)
            }
        };
        accounts.retain(|account| {
            !account
                .get("deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && (args.include_closed
                    || !account
                        .get("closed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false))
        });
        Ok(json!({
            "plan_id": plan_id,
            "server_knowledge": server_knowledge,
            "accounts": accounts,
        }))
    }

    async fn list_categories_inner(&self, args: ListCategoriesArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let month = match args.month.as_deref() {
            Some(month) => normalize_month(month)?,
            None => "current".to_string(),
        };
        let path = if month == "current" && args.month.is_none() {
            format!("plans/{plan_id}/categories")
        } else {
            format!("plans/{plan_id}/months/{month}")
        };
        let body = self
            .client
            .get_optional(&path, &[], CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        let Some(mut body) = body else {
            return Ok(json!({
                "plan_id": plan_id,
                "month": month,
                "categories": [],
            }));
        };
        let knowledge = body.pointer("/data/server_knowledge").cloned();
        present_money(&mut body);
        let mut categories = if body.pointer("/data/category_groups").is_some() {
            flatten_groups(&body)
        } else {
            body.pointer("/data/month/categories")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        categories.retain(|category| {
            !category
                .get("deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && (args.include_hidden
                    || !category
                        .get("hidden")
                        .and_then(Value::as_bool)
                        .unwrap_or(false))
                && query_matches(category, args.query.as_deref(), &["name"])
        });
        Ok(json!({
            "plan_id": plan_id,
            "month": month,
            "server_knowledge": knowledge,
            "categories": categories,
        }))
    }

    async fn month_summary_inner(&self, args: MonthSummaryArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let month = normalize_month(args.month.as_deref().unwrap_or("current"))?;
        let path = format!("plans/{plan_id}/months/{month}");
        let mut body = self
            .client
            .get(&path, &[], CacheMode::Bypass)
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut body);
        let month_value = body
            .pointer("/data/month")
            .cloned()
            .unwrap_or_else(|| json!({}));
        Ok(json!({
            "plan_id": plan_id,
            "month": month_value,
        }))
    }

    async fn list_transactions_inner(&self, args: ListTransactionsArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let since = optional_date("since_date", args.since_date.as_deref())?;
        let until = optional_date("until_date", args.until_date.as_deref())?;
        let account_id = optional_id("account_id", args.account_id.as_deref())?;
        let category_id = optional_id("category_id", args.category_id.as_deref())?;
        let payee_id = optional_id("payee_id", args.payee_id.as_deref())?;
        let path = if let Some(account_id) = &account_id {
            format!("plans/{plan_id}/accounts/{account_id}/transactions")
        } else if let Some(category_id) = &category_id {
            format!("plans/{plan_id}/categories/{category_id}/transactions")
        } else if let Some(payee_id) = &payee_id {
            format!("plans/{plan_id}/payees/{payee_id}/transactions")
        } else {
            format!("plans/{plan_id}/transactions")
        };
        let mut query: Vec<(String, String)> = Vec::new();
        if let Some(since) = &since {
            query.push(("since_date".into(), since.clone()));
        }
        if let Some(until) = &until {
            query.push(("until_date".into(), until.clone()));
        }
        if let Some(kind) = args.transaction_type {
            let label = serde_json::to_value(kind)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default();
            query.push(("type".into(), label));
        }
        let delta = args.since_server_knowledge.is_some();
        if let Some(knowledge) = args.since_server_knowledge {
            query.push(("last_knowledge_of_server".into(), knowledge.to_string()));
        }
        let query_ref: Vec<(&str, &str)> = query
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let cache = if delta {
            CacheMode::Bypass
        } else {
            CacheMode::Use
        };
        let fetched = self
            .client
            .get_optional(&path, &query_ref, cache)
            .await
            .map_err(|error| error.to_string())?;
        let (mut transactions, server_knowledge) = match fetched {
            None => (Vec::new(), None),
            Some(mut body) => {
                let knowledge = body.pointer("/data/server_knowledge").cloned();
                present_money(&mut body);
                let transactions = body
                    .pointer("/data/transactions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                (transactions, knowledge)
            }
        };
        let include_deleted = args.include_deleted || delta;
        if !include_deleted {
            transactions
                .retain(|txn| !txn.get("deleted").and_then(Value::as_bool).unwrap_or(false));
        }
        if account_id.is_some() {
            if let Some(category_id) = &category_id {
                transactions.retain(|txn| {
                    txn.get("category_id").and_then(Value::as_str) == Some(category_id.as_str())
                });
            }
        }
        if account_id.is_some() || category_id.is_some() {
            if let Some(payee_id) = &payee_id {
                transactions.retain(|txn| {
                    txn.get("payee_id").and_then(Value::as_str) == Some(payee_id.as_str())
                });
            }
        }
        if let Some(query) = args
            .query
            .as_deref()
            .filter(|query| !query.trim().is_empty())
        {
            transactions.retain(|txn| transaction_matches(txn, query));
        }
        transactions.sort_by(|left, right| {
            let left_date = left.get("date").and_then(Value::as_str).unwrap_or("");
            let right_date = right.get("date").and_then(Value::as_str).unwrap_or("");
            right_date.cmp(left_date).then_with(|| {
                let left_id = left.get("id").and_then(Value::as_str).unwrap_or("");
                let right_id = right.get("id").and_then(Value::as_str).unwrap_or("");
                right_id.cmp(left_id)
            })
        });
        let limit = args.limit.clamp(1, 200) as usize;
        let truncated = transactions.len() > limit;
        transactions.truncate(limit);
        Ok(json!({
            "plan_id": plan_id,
            "server_knowledge": server_knowledge,
            "count": transactions.len(),
            "truncated": truncated,
            "transactions": transactions,
        }))
    }

    async fn list_scheduled_inner(&self, args: PlanArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let path = format!("plans/{plan_id}/scheduled_transactions");
        let fetched = self
            .client
            .get_optional(&path, &[], CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        let (transactions, knowledge) = match fetched {
            None => (Vec::new(), None),
            Some(mut body) => {
                let knowledge = body.pointer("/data/server_knowledge").cloned();
                present_money(&mut body);
                let transactions = body
                    .pointer("/data/scheduled_transactions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|txn| !txn.get("deleted").and_then(Value::as_bool).unwrap_or(false))
                    .collect();
                (transactions, knowledge)
            }
        };
        Ok(json!({
            "plan_id": plan_id,
            "server_knowledge": knowledge,
            "scheduled_transactions": transactions,
        }))
    }

    async fn list_payees_inner(&self, args: ListPayeesArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let path = format!("plans/{plan_id}/payees");
        let fetched = self
            .client
            .get_optional(&path, &[], CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        let (payees, knowledge) = match fetched {
            None => (Vec::new(), None),
            Some(body) => {
                let knowledge = body.pointer("/data/server_knowledge").cloned();
                let payees = body
                    .pointer("/data/payees")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|payee| {
                        !payee
                            .get("deleted")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                            && (args.include_transfers
                                || payee
                                    .get("transfer_account_id")
                                    .and_then(Value::as_str)
                                    .is_none())
                            && query_matches(payee, args.query.as_deref(), &["name"])
                    })
                    .collect();
                (payees, knowledge)
            }
        };
        Ok(json!({
            "plan_id": plan_id,
            "server_knowledge": knowledge,
            "payees": payees,
        }))
    }

    async fn create_transaction_inner(&self, args: CreateTransactionArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let path = format!("plans/{plan_id}/transactions");
        let body = match (args.transaction, args.transactions) {
            (Some(transaction), None) => {
                json!({"transaction": transaction_body(&transaction, true)?})
            }
            (None, Some(transactions)) if !transactions.is_empty() => {
                let mut encoded = Vec::with_capacity(transactions.len());
                for transaction in &transactions {
                    encoded.push(transaction_body(transaction, true)?);
                }
                json!({"transactions": encoded})
            }
            (Some(_), Some(_)) => {
                return Err("pass either transaction or transactions, not both".into());
            }
            _ => return Err("transaction or transactions is required".into()),
        };
        let mut response = self
            .client
            .post(&path, body)
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(
            json!({"plan_id": plan_id, "result": response.pointer("/data").cloned().unwrap_or(response)}),
        )
    }

    async fn update_transaction_inner(&self, args: UpdateTransactionArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        if let Some(transactions) = args.transactions.filter(|items| !items.is_empty()) {
            let mut encoded = Vec::with_capacity(transactions.len());
            for transaction in &transactions {
                encoded.push(transaction_patch_body(transaction)?);
            }
            let path = format!("plans/{plan_id}/transactions");
            let mut response = self
                .client
                .patch(&path, json!({"transactions": encoded}))
                .await
                .map_err(|error| error.to_string())?;
            present_money(&mut response);
            return Ok(json!({
                "plan_id": plan_id,
                "result": response.pointer("/data").cloned().unwrap_or(response),
            }));
        }
        let transaction_id = args
            .transaction_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "transaction_id or transactions is required".to_string())?;
        validate_path_id("transaction_id", transaction_id)?;
        let patch = TransactionPatch {
            id: transaction_id.to_string(),
            account_id: args.account_id,
            date: args.date,
            amount: args.amount,
            payee_id: args.payee_id,
            payee_name: args.payee_name,
            category_id: args.category_id,
            memo: args.memo,
            cleared: args.cleared,
            approved: args.approved,
            flag_color: args.flag_color,
            clear_flag: args.clear_flag,
        };
        let mut fields = transaction_patch_body(&patch)?;
        fields
            .as_object_mut()
            .ok_or_else(|| "invalid update".to_string())?
            .remove("id");
        if fields.as_object().is_some_and(Map::is_empty) {
            return Err("provide at least one field to change".into());
        }
        let path = format!("plans/{plan_id}/transactions/{transaction_id}");
        let mut response = self
            .client
            .put(&path, json!({"transaction": fields}))
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(json!({
            "plan_id": plan_id,
            "result": response.pointer("/data").cloned().unwrap_or(response),
        }))
    }

    async fn delete_transaction_inner(&self, args: DeleteTransactionArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let transaction_id = args.transaction_id.trim();
        validate_path_id("transaction_id", transaction_id)?;
        let path = format!("plans/{plan_id}/transactions/{transaction_id}");
        let mut response = self
            .client
            .delete(&path)
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(json!({
            "plan_id": plan_id,
            "deleted_transaction_id": transaction_id,
            "result": response.pointer("/data").cloned().unwrap_or(response),
        }))
    }

    async fn set_approval_inner(&self, args: ApprovalArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let mut ids = Vec::new();
        if let Some(id) = args.transaction_id {
            ids.push(id);
        }
        if let Some(more) = args.transaction_ids {
            ids.extend(more);
        }
        if ids.is_empty() {
            return Err("transaction_id or transaction_ids is required".into());
        }
        let mut transactions = Vec::with_capacity(ids.len());
        for id in ids {
            let id = id.trim().to_string();
            validate_path_id("transaction_id", &id)?;
            transactions.push(json!({"id": id, "approved": args.approved}));
        }
        let path = format!("plans/{plan_id}/transactions");
        let mut response = self
            .client
            .patch(&path, json!({"transactions": transactions}))
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(json!({
            "plan_id": plan_id,
            "approved": args.approved,
            "result": response.pointer("/data").cloned().unwrap_or(response),
        }))
    }

    async fn create_category_group_inner(
        &self,
        args: CreateCategoryGroupArgs,
    ) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let name = require_trimmed("name", &args.name, 50)?;
        let path = format!("plans/{plan_id}/category_groups");
        let mut response = self
            .client
            .post(&path, json!({"category_group": {"name": name}}))
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(json!({
            "plan_id": plan_id,
            "result": response.pointer("/data").cloned().unwrap_or(response),
        }))
    }

    async fn create_category_inner(&self, args: CreateCategoryArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let category_group_id = args.category_group_id.trim();
        validate_path_id("category_group_id", category_group_id)?;
        let name = require_trimmed("name", &args.name, 200)?;
        let mut category = Map::new();
        category.insert("name".into(), json!(name));
        category.insert("category_group_id".into(), json!(category_group_id));
        if let Some(note) = args.note.as_deref() {
            let note = note.trim();
            if !note.is_empty() {
                check_len("note", note, 500)?;
                category.insert("note".into(), json!(note));
            }
        }
        if let Some(target) = &args.goal_target {
            let milliunits = target.to_milliunits()?;
            if milliunits < 0 {
                return Err("goal_target must be zero or a positive currency amount".into());
            }
            category.insert("goal_target".into(), json!(milliunits));
        }
        if let Some(date) = args.goal_target_date.as_deref() {
            let date = date.trim();
            if !date.is_empty() {
                category.insert("goal_target_date".into(), json!(normalize_date(date)?));
            }
        }
        if let Some(whole) = args.goal_needs_whole_amount {
            category.insert("goal_needs_whole_amount".into(), json!(whole));
        }
        let path = format!("plans/{plan_id}/categories");
        let mut response = self
            .client
            .post(&path, json!({"category": Value::Object(category)}))
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(json!({
            "plan_id": plan_id,
            "result": response.pointer("/data").cloned().unwrap_or(response),
        }))
    }

    async fn update_category_inner(&self, args: UpdateCategoryArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let category_id = args.category_id.trim();
        validate_path_id("category_id", category_id)?;
        let mut category = Map::new();
        if let Some(name) = args.name.as_deref() {
            category.insert("name".into(), json!(require_trimmed("name", name, 200)?));
        }
        if let Some(note) = args.note.as_deref() {
            let note = note.trim();
            if note.is_empty() {
                category.insert("note".into(), Value::Null);
            } else {
                check_len("note", note, 500)?;
                category.insert("note".into(), json!(note));
            }
        }
        if let Some(group_id) = optional_id("category_group_id", args.category_group_id.as_deref())?
        {
            category.insert("category_group_id".into(), json!(group_id));
        }
        if let Some(target) = &args.goal_target {
            let milliunits = target.to_milliunits()?;
            if milliunits < 0 {
                return Err("goal_target must be zero or a positive currency amount".into());
            }
            category.insert("goal_target".into(), json!(milliunits));
        }
        if let Some(date) = args.goal_target_date.as_deref() {
            let date = date.trim();
            if !date.is_empty() {
                category.insert("goal_target_date".into(), json!(normalize_date(date)?));
            }
        }
        if let Some(whole) = args.goal_needs_whole_amount {
            category.insert("goal_needs_whole_amount".into(), json!(whole));
        }
        if let Some(frequency) = args.goal_frequency {
            if !category.contains_key("goal_target") {
                return Err("goal_frequency requires goal_target".into());
            }
            if category.contains_key("goal_target_date") {
                return Err("goal_frequency cannot be combined with goal_target_date".into());
            }
            category.insert(
                "goal_frequency".into(),
                serde_json::to_value(frequency).unwrap_or(Value::Null),
            );
        }
        if category.is_empty() {
            return Err("provide at least one field to change".into());
        }
        let path = format!("plans/{plan_id}/categories/{category_id}");
        let mut response = self
            .client
            .patch(&path, json!({"category": Value::Object(category)}))
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(write_result(&plan_id, response))
    }

    async fn update_category_group_inner(
        &self,
        args: UpdateCategoryGroupArgs,
    ) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let category_group_id = args.category_group_id.trim();
        validate_path_id("category_group_id", category_group_id)?;
        let name = require_trimmed("name", &args.name, 50)?;
        let path = format!("plans/{plan_id}/category_groups/{category_group_id}");
        let mut response = self
            .client
            .patch(&path, json!({"category_group": {"name": name}}))
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(write_result(&plan_id, response))
    }

    async fn create_account_inner(&self, args: CreateAccountArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let name = require_trimmed("name", &args.name, 200)?;
        let path = format!("plans/{plan_id}/accounts");
        let mut response = self
            .client
            .post(
                &path,
                json!({
                    "account": {
                        "name": name,
                        "type": args.account_type,
                        "balance": args.balance.to_milliunits()?,
                    }
                }),
            )
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(write_result(&plan_id, response))
    }

    async fn create_payee_inner(&self, args: CreatePayeeArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let name = require_trimmed("name", &args.name, 500)?;
        let path = format!("plans/{plan_id}/payees");
        let mut response = self
            .client
            .post(&path, json!({"payee": {"name": name}}))
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(write_result(&plan_id, response))
    }

    async fn update_payee_inner(&self, args: UpdatePayeeArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let payee_id = args.payee_id.trim();
        validate_path_id("payee_id", payee_id)?;
        let name = require_trimmed("name", &args.name, 500)?;
        let path = format!("plans/{plan_id}/payees/{payee_id}");
        let mut response = self
            .client
            .patch(&path, json!({"payee": {"name": name}}))
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(write_result(&plan_id, response))
    }

    async fn import_transactions_inner(&self, args: PlanArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let path = format!("plans/{plan_id}/transactions/import");
        let mut response = self
            .client
            .post_without_body(&path)
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(write_result(&plan_id, response))
    }

    async fn list_months_inner(&self, args: PlanArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let path = format!("plans/{plan_id}/months");
        let fetched = self
            .client
            .get_optional(&path, &[], CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        let (months, knowledge) = match fetched {
            None => (Vec::new(), None),
            Some(mut body) => {
                let knowledge = body.pointer("/data/server_knowledge").cloned();
                present_money(&mut body);
                let months = body
                    .pointer("/data/months")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|month| {
                        !month
                            .get("deleted")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    })
                    .collect();
                (months, knowledge)
            }
        };
        Ok(json!({
            "plan_id": plan_id,
            "server_knowledge": knowledge,
            "months": months,
        }))
    }

    async fn get_category_inner(&self, args: GetCategoryArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let category_id = args.category_id.trim();
        validate_path_id("category_id", category_id)?;
        let path = format!("plans/{plan_id}/categories/{category_id}");
        let mut body = self
            .client
            .get(&path, &[], CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut body);
        Ok(json!({
            "plan_id": plan_id,
            "category": body.pointer("/data/category").cloned().unwrap_or(Value::Null),
        }))
    }

    async fn get_user_inner(&self, _args: GetUserArgs) -> Result<Value, String> {
        let body = self
            .client
            .get("user", &[], CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "user": body.pointer("/data/user").cloned().unwrap_or(Value::Null),
        }))
    }

    async fn get_plan_inner(&self, args: GetPlanArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let mut query: Vec<(String, String)> = Vec::new();
        if let Some(knowledge) = args.since_server_knowledge {
            query.push(("last_knowledge_of_server".into(), knowledge.to_string()));
        }
        let query_ref: Vec<(&str, &str)> = query
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let cache = if args.since_server_knowledge.is_some() {
            CacheMode::Bypass
        } else {
            CacheMode::Use
        };
        let path = format!("plans/{plan_id}");
        let mut body = self
            .client
            .get(&path, &query_ref, cache)
            .await
            .map_err(|error| error.to_string())?;
        let knowledge = body.pointer("/data/server_knowledge").cloned();
        present_money(&mut body);
        Ok(json!({
            "plan_id": plan_id,
            "server_knowledge": knowledge,
            "plan": body.pointer("/data/plan").cloned().unwrap_or(Value::Null),
        }))
    }

    async fn get_plan_settings_inner(&self, args: PlanArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let path = format!("plans/{plan_id}/settings");
        let body = self
            .client
            .get(&path, &[], CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "plan_id": plan_id,
            "settings": body.pointer("/data/settings").cloned().unwrap_or(Value::Null),
        }))
    }

    async fn list_money_movements_inner(
        &self,
        args: ListMoneyMovementsArgs,
    ) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let month = match args.month.as_deref() {
            Some(month) => Some(normalize_month(month)?),
            None => None,
        };
        let path = match &month {
            Some(month) => format!("plans/{plan_id}/months/{month}/money_movements"),
            None => format!("plans/{plan_id}/money_movements"),
        };
        let fetched = self
            .client
            .get_optional(&path, &[], CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        let (movements, knowledge) = match fetched {
            None => (Vec::new(), None),
            Some(mut body) => {
                let knowledge = body.pointer("/data/server_knowledge").cloned();
                present_money(&mut body);
                let movements = body
                    .pointer("/data/money_movements")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                (movements, knowledge)
            }
        };
        Ok(json!({
            "plan_id": plan_id,
            "month": month,
            "server_knowledge": knowledge,
            "money_movements": movements,
        }))
    }

    async fn get_transaction_inner(&self, args: GetTransactionArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let transaction_id = args.transaction_id.trim();
        validate_path_id("transaction_id", transaction_id)?;
        let path = format!("plans/{plan_id}/transactions/{transaction_id}");
        let mut body = self
            .client
            .get(&path, &[], CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        let knowledge = body.pointer("/data/server_knowledge").cloned();
        present_money(&mut body);
        Ok(json!({
            "plan_id": plan_id,
            "server_knowledge": knowledge,
            "transaction": body.pointer("/data/transaction").cloned().unwrap_or(Value::Null),
        }))
    }

    async fn get_scheduled_transaction_inner(
        &self,
        args: GetScheduledTransactionArgs,
    ) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let id = args.scheduled_transaction_id.trim();
        validate_path_id("scheduled_transaction_id", id)?;
        let path = format!("plans/{plan_id}/scheduled_transactions/{id}");
        let mut body = self
            .client
            .get(&path, &[], CacheMode::Use)
            .await
            .map_err(|error| error.to_string())?;
        let knowledge = body.pointer("/data/server_knowledge").cloned();
        present_money(&mut body);
        Ok(json!({
            "plan_id": plan_id,
            "server_knowledge": knowledge,
            "scheduled_transaction": body
                .pointer("/data/scheduled_transaction")
                .cloned()
                .unwrap_or(Value::Null),
        }))
    }

    async fn update_budget_inner(&self, args: UpdateCategoryBudgetArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let month = normalize_month(&args.month)?;
        let category_id = args.category_id.trim();
        validate_path_id("category_id", category_id)?;
        let current = self
            .category_budgeted(&plan_id, &month, category_id)
            .await?;
        let next = match (args.assigned, args.adjust_by) {
            (Some(assigned), None) => assigned.to_milliunits()?,
            (None, Some(adjust_by)) => current
                .checked_add(adjust_by.to_milliunits()?)
                .ok_or_else(|| "assigned amount overflowed".to_string())?,
            _ => {
                return Err("set exactly one of assigned (absolute) or adjust_by (delta)".into());
            }
        };
        let updated = self
            .set_budgeted(&plan_id, &month, category_id, next)
            .await?;
        Ok(json!({
            "plan_id": plan_id,
            "month": month,
            "category_id": category_id,
            "previous_assigned": milliunits_to_currency(current),
            "assigned": milliunits_to_currency(next),
            "category": updated,
        }))
    }

    async fn move_money_inner(&self, args: MoveMoneyArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let month = normalize_month(&args.month)?;
        let from_id = optional_id("from_category_id", args.from_category_id.as_deref())?;
        let to_id = optional_id("to_category_id", args.to_category_id.as_deref())?;
        if from_id.is_none() && to_id.is_none() {
            return Err(
                "set from_category_id, to_category_id, or both. Omit one side to use Ready to Assign"
                    .into(),
            );
        }
        if from_id.is_some() && from_id == to_id {
            return Err("from_category_id and to_category_id must be different".into());
        }
        let amount = args.amount.to_milliunits()?;
        if amount <= 0 {
            return Err("amount must be a positive currency amount".into());
        }
        let mut from_result = None;
        let mut to_result = None;
        if let Some(from_id) = &from_id {
            let current = self.category_budgeted(&plan_id, &month, from_id).await?;
            let next = current
                .checked_sub(amount)
                .ok_or_else(|| "assigned amount underflowed".to_string())?;
            from_result = Some(self.set_budgeted(&plan_id, &month, from_id, next).await?);
        }
        if let Some(to_id) = &to_id {
            let current = match self.category_budgeted(&plan_id, &month, to_id).await {
                Ok(current) => current,
                Err(error) if from_result.is_some() => {
                    return Err(format!(
                        "Money was removed from the source category, but the destination could not be read: {error}. Check both categories before retrying."
                    ));
                }
                Err(error) => return Err(error),
            };
            let next = current
                .checked_add(amount)
                .ok_or_else(|| "assigned amount overflowed".to_string())?;
            match self.set_budgeted(&plan_id, &month, to_id, next).await {
                Ok(updated) => to_result = Some(updated),
                Err(error) if from_result.is_some() => {
                    return Err(format!(
                        "Money was removed from the source category, but assigning it to the destination failed: {error}. The source category was already updated; check both categories before retrying."
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(json!({
            "plan_id": plan_id,
            "month": month,
            "amount": milliunits_to_currency(amount),
            "from_category_id": from_id,
            "to_category_id": to_id,
            "from_category": from_result,
            "to_category": to_result,
        }))
    }

    async fn create_scheduled_inner(&self, args: CreateScheduledArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let body = scheduled_body(&args.scheduled_transaction)?;
        let path = format!("plans/{plan_id}/scheduled_transactions");
        let mut response = self
            .client
            .post(&path, json!({"scheduled_transaction": body}))
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(json!({
            "plan_id": plan_id,
            "result": response.pointer("/data").cloned().unwrap_or(response),
        }))
    }

    async fn update_scheduled_inner(&self, args: UpdateScheduledArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id.clone())?;
        let id = args.scheduled_transaction_id.trim();
        validate_path_id("scheduled_transaction_id", id)?;
        let path = format!("plans/{plan_id}/scheduled_transactions/{id}");
        let existing = self
            .client
            .get(&path, &[], CacheMode::Bypass)
            .await
            .map_err(|error| error.to_string())?;
        let body = merge_scheduled(
            existing
                .pointer("/data/scheduled_transaction")
                .ok_or_else(|| {
                    "YNAB response did not include the scheduled transaction".to_string()
                })?,
            &args,
        )?;
        let mut response = self
            .client
            .put(&path, json!({"scheduled_transaction": body}))
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(json!({
            "plan_id": plan_id,
            "result": response.pointer("/data").cloned().unwrap_or(response),
        }))
    }

    async fn delete_scheduled_inner(&self, args: DeleteScheduledArgs) -> Result<Value, String> {
        let plan_id = self.plan_id(args.plan_id)?;
        let id = args.scheduled_transaction_id.trim();
        validate_path_id("scheduled_transaction_id", id)?;
        let path = format!("plans/{plan_id}/scheduled_transactions/{id}");
        let mut response = self
            .client
            .delete(&path)
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(json!({
            "plan_id": plan_id,
            "deleted_scheduled_transaction_id": id,
            "result": response.pointer("/data").cloned().unwrap_or(response),
        }))
    }

    async fn category_budgeted(
        &self,
        plan_id: &str,
        month: &str,
        category_id: &str,
    ) -> Result<i64, String> {
        let path = format!("plans/{plan_id}/months/{month}/categories/{category_id}");
        let body = self
            .client
            .get(&path, &[], CacheMode::Bypass)
            .await
            .map_err(|error| error.to_string())?;
        body.pointer("/data/category/budgeted")
            .and_then(Value::as_i64)
            .ok_or_else(|| "YNAB did not return the category assigned amount".to_string())
    }

    async fn set_budgeted(
        &self,
        plan_id: &str,
        month: &str,
        category_id: &str,
        budgeted: i64,
    ) -> Result<Value, String> {
        let path = format!("plans/{plan_id}/months/{month}/categories/{category_id}");
        let mut response = self
            .client
            .patch(&path, json!({"category": {"budgeted": budgeted}}))
            .await
            .map_err(|error| error.to_string())?;
        present_money(&mut response);
        Ok(response
            .pointer("/data/category")
            .cloned()
            .unwrap_or(response))
    }
}

#[tool_router]
impl YnabServer {
    #[tool(
        description = "List YNAB plans (budgets) the access token can see, including ids and currency.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn list_plans(
        &self,
        Parameters(args): Parameters<ListPlansArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.list_plans_inner(args).await)
    }

    #[tool(
        description = "List accounts in a plan. Balances are currency units. transfer_payee_id is the payee to use when transferring into that account.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn list_accounts(
        &self,
        Parameters(args): Parameters<ListAccountsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.list_accounts_inner(args).await)
    }

    #[tool(
        description = "List categories with assigned (budgeted), activity, and available amounts in currency units for a month.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn list_categories(
        &self,
        Parameters(args): Parameters<ListCategoriesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.list_categories_inner(args).await)
    }

    #[tool(
        description = "Month summary: income, assigned, activity, Ready to Assign, and each category's budgeted, activity, and available amounts.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn get_month_summary(
        &self,
        Parameters(args): Parameters<MonthSummaryArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.month_summary_inner(args).await)
    }

    #[tool(
        description = "List or search transactions. Amounts are currency units. Filter by date, account, category, payee, or a text query over payee, category, memo, and account name.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn list_transactions(
        &self,
        Parameters(args): Parameters<ListTransactionsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.list_transactions_inner(args).await)
    }

    #[tool(
        description = "List upcoming and recurring scheduled transactions. Amounts are currency units.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn list_scheduled_transactions(
        &self,
        Parameters(args): Parameters<PlanArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.list_scheduled_inner(args).await)
    }

    #[tool(
        description = "List payees. Transfer payees have transfer_account_id set; use that payee id to transfer into the account.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn list_payees(
        &self,
        Parameters(args): Parameters<ListPayeesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.list_payees_inner(args).await)
    }

    #[tool(
        description = "Create one transaction or several. Amounts are currency units; outflows are negative. Created transactions are approved unless approved is false. Future dates belong on create_scheduled_transaction.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn create_transaction(
        &self,
        Parameters(args): Parameters<CreateTransactionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.create_transaction_inner(args).await)
    }

    #[tool(
        description = "Update one transaction by transaction_id, or several via transactions. Omitted fields stay unchanged. Amounts are currency units. An empty category_id or memo clears that field.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_transaction(
        &self,
        Parameters(args): Parameters<UpdateTransactionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.update_transaction_inner(args).await)
    }

    #[tool(
        description = "Delete one transaction from the plan.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn delete_transaction(
        &self,
        Parameters(args): Parameters<DeleteTransactionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.delete_transaction_inner(args).await)
    }

    #[tool(
        description = "Approve or unapprove one or more transactions. YNAB has no separate approve endpoint; this updates the approved flag.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn set_transaction_approval(
        &self,
        Parameters(args): Parameters<ApprovalArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.set_approval_inner(args).await)
    }

    #[tool(
        description = "Create a category group. name is required and at most 50 characters. Use the returned category group id as category_group_id in create_category.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn create_category_group(
        &self,
        Parameters(args): Parameters<CreateCategoryGroupArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.create_category_group_inner(args).await)
    }

    #[tool(
        description = "Create a category in an existing group. Requires name and category_group_id (from list_categories or create_category_group). Optional note, goal_target (currency units), goal_target_date (YYYY-MM-DD), and goal_needs_whole_amount. YNAB rejects internal groups such as Credit Card Payments.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn create_category(
        &self,
        Parameters(args): Parameters<CreateCategoryArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.create_category_inner(args).await)
    }

    #[tool(
        description = "Update a category's name, note, group, or goal. Omitted fields stay unchanged. An empty note clears it. goal_target is currency units. goal_frequency (monthly, weekly, yearly) requires goal_target and cannot be combined with goal_target_date. This does not change the assigned amount; use update_category_budget for that.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_category(
        &self,
        Parameters(args): Parameters<UpdateCategoryArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.update_category_inner(args).await)
    }

    #[tool(
        description = "Rename a category group. name is required and at most 50 characters.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_category_group(
        &self,
        Parameters(args): Parameters<UpdateCategoryGroupArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.update_category_group_inner(args).await)
    }

    #[tool(
        description = "Create an account. type is checking, savings, cash, creditCard, otherAsset, or otherLiability. balance is the starting balance in currency units.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn create_account(
        &self,
        Parameters(args): Parameters<CreateAccountArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.create_account_inner(args).await)
    }

    #[tool(
        description = "Create a payee. name is required and at most 500 characters. YNAB also creates a payee when a transaction uses a new payee_name.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn create_payee(
        &self,
        Parameters(args): Parameters<CreatePayeeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.create_payee_inner(args).await)
    }

    #[tool(
        description = "Rename a payee. name is required and at most 500 characters.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_payee(
        &self,
        Parameters(args): Parameters<UpdatePayeeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.update_payee_inner(args).await)
    }

    #[tool(
        description = "Import available transactions from every linked account on the plan. Same as Import in YNAB. Returns the imported transaction ids. There is no request body.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn import_transactions(
        &self,
        Parameters(args): Parameters<PlanArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.import_transactions_inner(args).await)
    }

    #[tool(
        description = "List plan months with income, assigned, activity, and Ready to Assign in currency units. Deleted months are omitted.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn list_months(
        &self,
        Parameters(args): Parameters<PlanArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.list_months_inner(args).await)
    }

    #[tool(
        description = "Get one category for the current plan month, including assigned, activity, available, and goal fields in currency units.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn get_category(
        &self,
        Parameters(args): Parameters<GetCategoryArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.get_category_inner(args).await)
    }

    #[tool(
        description = "Get the YNAB user id for the access token. This call does not use a plan.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn get_user(
        &self,
        Parameters(args): Parameters<GetUserArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.get_user_inner(args).await)
    }

    #[tool(
        description = "Get one plan, including its accounts, categories, payees, months, and transactions. This is a full export and costs one request; prefer the narrower list tools. since_server_knowledge returns only changes since that cursor.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn get_plan(
        &self,
        Parameters(args): Parameters<GetPlanArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.get_plan_inner(args).await)
    }

    #[tool(
        description = "Get a plan's date format and currency format.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn get_plan_settings(
        &self,
        Parameters(args): Parameters<PlanArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.get_plan_settings_inner(args).await)
    }

    #[tool(
        description = "List money movements between categories, or between a category and Ready to Assign. Amounts are currency units. Pass month (YYYY-MM or current) to limit to one month.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn list_money_movements(
        &self,
        Parameters(args): Parameters<ListMoneyMovementsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.list_money_movements_inner(args).await)
    }

    #[tool(
        description = "Get one transaction by id. Amounts are currency units.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn get_transaction(
        &self,
        Parameters(args): Parameters<GetTransactionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.get_transaction_inner(args).await)
    }

    #[tool(
        description = "Get one scheduled transaction by id. Amounts are currency units.",
        annotations(
            read_only_hint = true,
            open_world_hint = true,
            destructive_hint = false
        )
    )]
    async fn get_scheduled_transaction(
        &self,
        Parameters(args): Parameters<GetScheduledTransactionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.get_scheduled_transaction_inner(args).await)
    }

    #[tool(
        description = "Set or adjust a category's assigned amount for a month. assigned replaces the amount; adjust_by adds a currency delta. This is how money is assigned from Ready to Assign.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_category_budget(
        &self,
        Parameters(args): Parameters<UpdateCategoryBudgetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.update_budget_inner(args).await)
    }

    #[tool(
        description = "Move a positive currency amount between categories in a month, or between a category and Ready to Assign. Omit from_category_id to take from Ready to Assign, or to_category_id to return money there.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn move_money(
        &self,
        Parameters(args): Parameters<MoveMoneyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.move_money_inner(args).await)
    }

    #[tool(
        description = "Create a scheduled or recurring transaction on a future date. Amounts are currency units. Splits are not supported by YNAB.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn create_scheduled_transaction(
        &self,
        Parameters(args): Parameters<CreateScheduledArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.create_scheduled_inner(args).await)
    }

    #[tool(
        description = "Update a scheduled transaction. Omitted fields keep their current values. Amounts are currency units. date, when set, must be in the future.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn update_scheduled_transaction(
        &self,
        Parameters(args): Parameters<UpdateScheduledArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.update_scheduled_inner(args).await)
    }

    #[tool(
        description = "Delete a scheduled transaction so it will not be entered.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn delete_scheduled_transaction(
        &self,
        Parameters(args): Parameters<DeleteScheduledArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(self.delete_scheduled_inner(args).await)
    }
}

#[tool_handler(
    name = "ynab",
    instructions = "Family YNAB budget server. Amounts are currency units such as euros, never milliunits: expenses are negative (for example -12.50) and income is positive. Call list_plans when plan_id is unknown; otherwise omit plan_id to use the server default. Months accept YYYY-MM or current. list_transactions searches payee, category, and memo. Writes change the live plan. The YNAB API allows about 200 requests per hour, so filter by date and reuse ids from earlier reads. Scheduled transactions must have a future date. To transfer between accounts, use the destination account transfer_payee_id as payee_id. To add envelopes, call create_category_group, then create_category with that group's id. category_group_id is also on each category from list_categories. update_category changes name, note, group, or goal; update_category_budget changes the assigned amount. create_account needs name, type, and balance. import_transactions pulls linked-account transactions. get_plan is a full export; prefer narrower list tools."
)]
impl ServerHandler for YnabServer {}

fn write_result(plan_id: &str, response: Value) -> Value {
    json!({
        "plan_id": plan_id,
        "result": response.pointer("/data").cloned().unwrap_or(response),
    })
}

fn finish(result: Result<Value, String>) -> Result<CallToolResult, ErrorData> {
    match result {
        Ok(value) => Ok(CallToolResult::structured(value)),
        Err(message) => Ok(CallToolResult::error(vec![ContentBlock::text(message)])),
    }
}

fn flatten_groups(body: &Value) -> Vec<Value> {
    let Some(groups) = body
        .pointer("/data/category_groups")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut categories = Vec::new();
    for group in groups {
        let Some(items) = group.get("categories").and_then(Value::as_array) else {
            continue;
        };
        if group
            .get("deleted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        categories.extend(items.iter().cloned());
    }
    categories
}

fn query_matches(value: &Value, query: Option<&str>, fields: &[&str]) -> bool {
    let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) else {
        return true;
    };
    let needle = query.to_lowercase();
    fields.iter().any(|field| {
        value
            .get(*field)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase()
            .contains(&needle)
    })
}

fn transaction_matches(transaction: &Value, query: &str) -> bool {
    const FIELDS: &[&str] = &[
        "payee_name",
        "category_name",
        "memo",
        "account_name",
        "amount_formatted",
    ];
    if query_matches(transaction, Some(query), FIELDS) {
        return true;
    }
    let needle = query.trim();
    if transaction
        .get("amount")
        .and_then(Value::as_f64)
        .is_some_and(|amount| amount.to_string().contains(needle))
    {
        return true;
    }
    transaction
        .get("subtransactions")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                query_matches(
                    item,
                    Some(query),
                    &["payee_name", "category_name", "memo", "amount_formatted"],
                )
            })
        })
}

fn require_trimmed(label: &str, value: &str, max: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{label} is required"));
    }
    check_len(label, value, max)?;
    Ok(value.to_string())
}

fn optional_date(label: &str, value: Option<&str>) -> Result<Option<String>, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => normalize_date(value)
            .map(Some)
            .map_err(|_| format!("{label} must be YYYY-MM-DD")),
    }
}

fn optional_id(label: &str, value: Option<&str>) -> Result<Option<String>, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => {
            validate_path_id(label, value)?;
            Ok(Some(value.to_string()))
        }
    }
}

fn transaction_body(input: &TransactionInput, default_approved: bool) -> Result<Value, String> {
    validate_path_id("account_id", input.account_id.trim())?;
    let date = normalize_date(&input.date)?;
    let amount = input.amount.to_milliunits()?;
    let mut object = Map::new();
    object.insert("account_id".into(), json!(input.account_id.trim()));
    object.insert("date".into(), json!(date));
    object.insert("amount".into(), json!(amount));
    insert_id(&mut object, "payee_id", input.payee_id.as_deref())?;
    insert_text(&mut object, "payee_name", input.payee_name.as_deref(), 200)?;
    insert_id(&mut object, "category_id", input.category_id.as_deref())?;
    insert_text(&mut object, "memo", input.memo.as_deref(), 500)?;
    if let Some(cleared) = input.cleared {
        object.insert(
            "cleared".into(),
            serde_json::to_value(cleared).unwrap_or(Value::Null),
        );
    }
    object.insert(
        "approved".into(),
        json!(input.approved.unwrap_or(default_approved)),
    );
    if let Some(flag) = input.flag_color {
        object.insert(
            "flag_color".into(),
            serde_json::to_value(flag).unwrap_or(Value::Null),
        );
    }
    if let Some(subs) = &input.subtransactions {
        if subs.is_empty() {
            return Err("subtransactions must not be empty when provided".into());
        }
        let mut encoded = Vec::with_capacity(subs.len());
        let mut sum = 0_i64;
        for sub in subs {
            let sub_amount = sub.amount.to_milliunits()?;
            sum = sum
                .checked_add(sub_amount)
                .ok_or_else(|| "split amounts overflowed".to_string())?;
            encoded.push(subtransaction_body(sub, sub_amount)?);
        }
        if sum != amount {
            return Err(format!(
                "split amounts must add up to the transaction amount ({}, got {})",
                milliunits_to_currency(amount),
                milliunits_to_currency(sum)
            ));
        }
        object.insert("category_id".into(), Value::Null);
        object.insert("subtransactions".into(), Value::Array(encoded));
    }
    Ok(Value::Object(object))
}

fn subtransaction_body(input: &SubtransactionInput, amount: i64) -> Result<Value, String> {
    let mut object = Map::new();
    object.insert("amount".into(), json!(amount));
    insert_id(&mut object, "payee_id", input.payee_id.as_deref())?;
    insert_text(&mut object, "payee_name", input.payee_name.as_deref(), 200)?;
    insert_id(&mut object, "category_id", input.category_id.as_deref())?;
    insert_text(&mut object, "memo", input.memo.as_deref(), 500)?;
    Ok(Value::Object(object))
}

fn transaction_patch_body(input: &TransactionPatch) -> Result<Value, String> {
    validate_path_id("id", input.id.trim())?;
    let mut object = Map::new();
    object.insert("id".into(), json!(input.id.trim()));
    if let Some(account_id) = input.account_id.as_deref() {
        validate_path_id("account_id", account_id.trim())?;
        object.insert("account_id".into(), json!(account_id.trim()));
    }
    if let Some(date) = input.date.as_deref() {
        object.insert("date".into(), json!(normalize_date(date)?));
    }
    if let Some(amount) = &input.amount {
        object.insert("amount".into(), json!(amount.to_milliunits()?));
    }
    insert_id(&mut object, "payee_id", input.payee_id.as_deref())?;
    insert_text(&mut object, "payee_name", input.payee_name.as_deref(), 200)?;
    insert_id(&mut object, "category_id", input.category_id.as_deref())?;
    insert_text(&mut object, "memo", input.memo.as_deref(), 500)?;
    if let Some(cleared) = input.cleared {
        object.insert(
            "cleared".into(),
            serde_json::to_value(cleared).unwrap_or(Value::Null),
        );
    }
    if let Some(approved) = input.approved {
        object.insert("approved".into(), json!(approved));
    }
    if input.clear_flag {
        object.insert("flag_color".into(), json!(""));
    } else if let Some(flag) = input.flag_color {
        object.insert(
            "flag_color".into(),
            serde_json::to_value(flag).unwrap_or(Value::Null),
        );
    }
    Ok(Value::Object(object))
}

fn scheduled_body(input: &ScheduledInput) -> Result<Value, String> {
    validate_path_id("account_id", input.account_id.trim())?;
    let date = normalize_date(&input.date)?;
    require_future_date(&date)?;
    let mut object = Map::new();
    object.insert("account_id".into(), json!(input.account_id.trim()));
    object.insert("date".into(), json!(date));
    object.insert("amount".into(), json!(input.amount.to_milliunits()?));
    object.insert(
        "frequency".into(),
        serde_json::to_value(input.frequency).unwrap_or(Value::Null),
    );
    insert_id(&mut object, "payee_id", input.payee_id.as_deref())?;
    insert_text(&mut object, "payee_name", input.payee_name.as_deref(), 200)?;
    insert_id(&mut object, "category_id", input.category_id.as_deref())?;
    insert_text(&mut object, "memo", input.memo.as_deref(), 500)?;
    if let Some(flag) = input.flag_color {
        object.insert(
            "flag_color".into(),
            serde_json::to_value(flag).unwrap_or(Value::Null),
        );
    }
    Ok(Value::Object(object))
}

fn merge_scheduled(existing: &Value, patch: &UpdateScheduledArgs) -> Result<Value, String> {
    let account_id = match patch.account_id.as_deref() {
        Some(account_id) => {
            validate_path_id("account_id", account_id.trim())?;
            account_id.trim().to_string()
        }
        None => existing
            .get("account_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "existing scheduled transaction has no account_id".to_string())?
            .to_string(),
    };
    let date = match patch.date.as_deref() {
        Some(date) => {
            let date = normalize_date(date)?;
            require_future_date(&date)?;
            date
        }
        None => existing
            .get("date_next")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                "existing scheduled transaction has no date_next; pass date".to_string()
            })?
            .to_string(),
    };
    let amount = match &patch.amount {
        Some(amount) => amount.to_milliunits()?,
        None => existing
            .get("amount")
            .and_then(Value::as_i64)
            .ok_or_else(|| "existing scheduled transaction has no amount".to_string())?,
    };
    let mut object = Map::new();
    object.insert("account_id".into(), json!(account_id));
    object.insert("date".into(), json!(date));
    object.insert("amount".into(), json!(amount));
    if let Some(frequency) = patch.frequency {
        object.insert(
            "frequency".into(),
            serde_json::to_value(frequency).unwrap_or(Value::Null),
        );
    } else if let Some(frequency) = existing.get("frequency").and_then(Value::as_str) {
        object.insert("frequency".into(), json!(frequency));
    }
    if patch.payee_name.is_some() && patch.payee_id.is_none() {
        insert_text(&mut object, "payee_name", patch.payee_name.as_deref(), 200)?;
    } else {
        let payee_id = patch
            .payee_id
            .as_deref()
            .or_else(|| existing.get("payee_id").and_then(Value::as_str));
        insert_id(&mut object, "payee_id", payee_id)?;
        insert_text(&mut object, "payee_name", patch.payee_name.as_deref(), 200)?;
    }
    let category_id = patch
        .category_id
        .as_deref()
        .or_else(|| existing.get("category_id").and_then(Value::as_str));
    insert_id(&mut object, "category_id", category_id)?;
    if let Some(memo) = patch.memo.as_deref() {
        insert_text(&mut object, "memo", Some(memo), 500)?;
    } else if let Some(memo) = existing.get("memo").and_then(Value::as_str) {
        object.insert("memo".into(), json!(memo));
    }
    if patch.clear_flag {
        object.insert("flag_color".into(), json!(""));
    } else if let Some(flag) = patch.flag_color {
        object.insert(
            "flag_color".into(),
            serde_json::to_value(flag).unwrap_or(Value::Null),
        );
    } else if let Some(flag) = existing.get("flag_color").and_then(Value::as_str) {
        object.insert("flag_color".into(), json!(flag));
    }
    Ok(Value::Object(object))
}

fn insert_id(
    object: &mut Map<String, Value>,
    key: &str,
    value: Option<&str>,
) -> Result<(), String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        if value.is_some() {
            object.insert(key.to_string(), Value::Null);
        }
        return Ok(());
    };
    validate_path_id(key, value)?;
    object.insert(key.to_string(), json!(value));
    Ok(())
}

fn insert_text(
    object: &mut Map<String, Value>,
    key: &str,
    value: Option<&str>,
    max: usize,
) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_empty() {
        object.insert(key.to_string(), Value::Null);
        return Ok(());
    }
    check_len(key, value, max)?;
    object.insert(key.to_string(), json!(value));
    Ok(())
}
