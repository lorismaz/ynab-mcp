use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::money::CurrencyAmount;

fn default_limit() -> u32 {
    50
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListPlansArgs {
    /// When true, each plan includes its accounts. Costs a larger response.
    #[serde(default)]
    pub include_accounts: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlanArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListAccountsArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Include closed accounts. Deleted accounts stay hidden.
    #[serde(default)]
    pub include_closed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListCategoriesArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// YYYY-MM, YYYY-MM-DD, or "current". Omit for the current month.
    #[serde(default)]
    pub month: Option<String>,
    /// Case-insensitive match on the category name.
    #[serde(default)]
    pub query: Option<String>,
    /// Include hidden categories.
    #[serde(default)]
    pub include_hidden: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MonthSummaryArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// YYYY-MM, YYYY-MM-DD, or "current". Defaults to current.
    #[serde(default)]
    pub month: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TransactionTypeFilter {
    Uncategorized,
    Unapproved,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTransactionsArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Inclusive start date, YYYY-MM-DD. YNAB defaults to one year ago when omitted.
    #[serde(default)]
    pub since_date: Option<String>,
    /// Inclusive end date, YYYY-MM-DD.
    #[serde(default)]
    pub until_date: Option<String>,
    /// Limit the YNAB query to one account.
    #[serde(default)]
    pub account_id: Option<String>,
    /// Limit the YNAB query to one category, or filter a fetched account.
    #[serde(default)]
    pub category_id: Option<String>,
    /// Limit the YNAB query to one payee.
    #[serde(default)]
    pub payee_id: Option<String>,
    /// Case-insensitive search over payee, category, memo, and account name.
    #[serde(default)]
    pub query: Option<String>,
    /// Ask YNAB for only uncategorized or unapproved transactions.
    #[serde(default)]
    pub transaction_type: Option<TransactionTypeFilter>,
    /// Delta cursor from a previous response's server_knowledge.
    #[serde(default)]
    pub since_server_knowledge: Option<i64>,
    /// Include deleted rows. Automatically included for delta requests.
    #[serde(default)]
    pub include_deleted: bool,
    /// Maximum rows after filtering. Default 50, maximum 200.
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListPayeesArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Case-insensitive match on the payee name.
    #[serde(default)]
    pub query: Option<String>,
    /// Include transfer payees (needed to move money between accounts).
    #[serde(default = "default_true")]
    pub include_transfers: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ClearedStatus {
    Cleared,
    Uncleared,
    Reconciled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub enum FlagColor {
    #[serde(rename = "red")]
    Red,
    #[serde(rename = "orange")]
    Orange,
    #[serde(rename = "yellow")]
    Yellow,
    #[serde(rename = "green")]
    Green,
    #[serde(rename = "blue")]
    Blue,
    #[serde(rename = "purple")]
    Purple,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub enum Frequency {
    #[serde(rename = "never")]
    Never,
    #[serde(rename = "daily")]
    Daily,
    #[serde(rename = "weekly")]
    Weekly,
    #[serde(rename = "everyOtherWeek")]
    EveryOtherWeek,
    #[serde(rename = "twiceAMonth")]
    TwiceAMonth,
    #[serde(rename = "every4Weeks")]
    Every4Weeks,
    #[serde(rename = "monthly")]
    Monthly,
    #[serde(rename = "everyOtherMonth")]
    EveryOtherMonth,
    #[serde(rename = "every3Months")]
    Every3Months,
    #[serde(rename = "every4Months")]
    Every4Months,
    #[serde(rename = "twiceAYear")]
    TwiceAYear,
    #[serde(rename = "yearly")]
    Yearly,
    #[serde(rename = "everyOtherYear")]
    EveryOtherYear,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubtransactionInput {
    /// Amount in currency units. Splits use the same sign as the parent.
    pub amount: CurrencyAmount,
    #[serde(default)]
    pub payee_id: Option<String>,
    #[serde(default)]
    pub payee_name: Option<String>,
    /// Category for this split. Credit-card payment categories are ignored by YNAB.
    #[serde(default)]
    pub category_id: Option<String>,
    #[serde(default)]
    pub memo: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TransactionInput {
    pub account_id: String,
    /// YYYY-MM-DD. Must not be in the future.
    pub date: String,
    /// Currency units. Negative for spending, positive for income.
    pub amount: CurrencyAmount,
    #[serde(default)]
    pub payee_id: Option<String>,
    /// Used when payee_id is omitted. YNAB matches an existing payee or creates one.
    #[serde(default)]
    pub payee_name: Option<String>,
    /// Category id. Leave empty and set subtransactions to create a split.
    #[serde(default)]
    pub category_id: Option<String>,
    #[serde(default)]
    pub memo: Option<String>,
    #[serde(default)]
    pub cleared: Option<ClearedStatus>,
    /// Defaults to true for transactions this server creates, so they do not wait for approval.
    #[serde(default)]
    pub approved: Option<bool>,
    #[serde(default)]
    pub flag_color: Option<FlagColor>,
    #[serde(default)]
    pub subtransactions: Option<Vec<SubtransactionInput>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTransactionArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// One transaction. Do not also set transactions.
    #[serde(default)]
    pub transaction: Option<TransactionInput>,
    /// Several transactions created in one YNAB call.
    #[serde(default)]
    pub transactions: Option<Vec<TransactionInput>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TransactionPatch {
    pub id: String,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub amount: Option<CurrencyAmount>,
    #[serde(default)]
    pub payee_id: Option<String>,
    #[serde(default)]
    pub payee_name: Option<String>,
    /// Empty string clears the category.
    #[serde(default)]
    pub category_id: Option<String>,
    /// Empty string clears the memo.
    #[serde(default)]
    pub memo: Option<String>,
    #[serde(default)]
    pub cleared: Option<ClearedStatus>,
    #[serde(default)]
    pub approved: Option<bool>,
    #[serde(default)]
    pub flag_color: Option<FlagColor>,
    #[serde(default)]
    pub clear_flag: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTransactionArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Update this one transaction. Do not also set transactions.
    #[serde(default)]
    pub transaction_id: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub amount: Option<CurrencyAmount>,
    #[serde(default)]
    pub payee_id: Option<String>,
    #[serde(default)]
    pub payee_name: Option<String>,
    #[serde(default)]
    pub category_id: Option<String>,
    #[serde(default)]
    pub memo: Option<String>,
    #[serde(default)]
    pub cleared: Option<ClearedStatus>,
    #[serde(default)]
    pub approved: Option<bool>,
    #[serde(default)]
    pub flag_color: Option<FlagColor>,
    #[serde(default)]
    pub clear_flag: bool,
    /// Update many transactions. Each item needs an id.
    #[serde(default)]
    pub transactions: Option<Vec<TransactionPatch>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteTransactionArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    pub transaction_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApprovalArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// One transaction to approve or unapprove.
    #[serde(default)]
    pub transaction_id: Option<String>,
    /// Several transactions changed in one call.
    #[serde(default)]
    pub transaction_ids: Option<Vec<String>>,
    /// True approves. False returns the transaction to the unapproved queue.
    pub approved: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateCategoryGroupArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Group name. YNAB allows at most 50 characters.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateCategoryArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Existing category group id from list_categories or create_category_group. Internal groups are rejected by YNAB.
    pub category_group_id: String,
    /// Category name, at most 200 characters.
    pub name: String,
    /// Optional note stored on the category.
    #[serde(default)]
    pub note: Option<String>,
    /// Optional goal target in currency units. When set, YNAB creates a monthly goal.
    #[serde(default)]
    pub goal_target: Option<CurrencyAmount>,
    /// Optional goal target date, YYYY-MM-DD.
    #[serde(default)]
    pub goal_target_date: Option<String>,
    /// For a Plan Your Spending goal: true sets aside the full target each period; false refills up to the target.
    #[serde(default)]
    pub goal_needs_whole_amount: Option<bool>,
}

/// Recurring NEED target cadence. YNAB rejects this together with goal_target_date.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub enum GoalFrequency {
    #[serde(rename = "monthly")]
    Monthly,
    #[serde(rename = "weekly")]
    Weekly,
    #[serde(rename = "yearly")]
    Yearly,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateCategoryArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    pub category_id: String,
    /// New name, at most 200 characters. Omit to leave the name unchanged.
    #[serde(default)]
    pub name: Option<String>,
    /// New note, at most 500 characters. An empty string clears the note. Omit to leave it unchanged.
    #[serde(default)]
    pub note: Option<String>,
    /// Move the category into this group. Internal groups are rejected by YNAB.
    #[serde(default)]
    pub category_group_id: Option<String>,
    /// Goal target in currency units. Required when goal_frequency is set. Omit to leave the target unchanged.
    #[serde(default)]
    pub goal_target: Option<CurrencyAmount>,
    /// Goal target date, YYYY-MM-DD. Cannot be combined with goal_frequency.
    #[serde(default)]
    pub goal_target_date: Option<String>,
    /// For a Plan Your Spending goal: true sets aside the full target each period; false refills up to the target.
    #[serde(default)]
    pub goal_needs_whole_amount: Option<bool>,
    /// Recurring target cadence: monthly, weekly, or yearly. Requires goal_target and replaces any existing target cadence.
    #[serde(default)]
    pub goal_frequency: Option<GoalFrequency>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateCategoryGroupArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    pub category_group_id: String,
    /// New group name. YNAB allows at most 50 characters.
    pub name: String,
}

/// Account types YNAB accepts when creating an account.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub enum AccountType {
    #[serde(rename = "checking")]
    Checking,
    #[serde(rename = "savings")]
    Savings,
    #[serde(rename = "cash")]
    Cash,
    #[serde(rename = "creditCard")]
    CreditCard,
    #[serde(rename = "otherAsset")]
    OtherAsset,
    #[serde(rename = "otherLiability")]
    OtherLiability,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateAccountArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Account name, at most 200 characters.
    pub name: String,
    /// checking, savings, cash, creditCard, otherAsset, or otherLiability.
    #[serde(rename = "type")]
    pub account_type: AccountType,
    /// Starting balance in currency units. Negative means the account owes money.
    pub balance: CurrencyAmount,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreatePayeeArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Payee name, at most 500 characters.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdatePayeeArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    pub payee_id: String,
    /// New payee name, at most 500 characters.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetCategoryArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    pub category_id: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GetUserArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetPlanArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Delta cursor from a previous response's server_knowledge. This response is a full plan export.
    #[serde(default)]
    pub since_server_knowledge: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMoneyMovementsArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// YYYY-MM, YYYY-MM-DD, or "current". Omit to list every month.
    #[serde(default)]
    pub month: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTransactionArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    pub transaction_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetScheduledTransactionArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    pub scheduled_transaction_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateCategoryBudgetArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// YYYY-MM, YYYY-MM-DD, or "current".
    pub month: String,
    pub category_id: String,
    /// Absolute assigned amount in currency units. Use this or adjust_by, not both.
    #[serde(default)]
    pub assigned: Option<CurrencyAmount>,
    /// Add this currency amount (negative to remove) to the current assigned amount.
    #[serde(default)]
    pub adjust_by: Option<CurrencyAmount>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveMoneyArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// YYYY-MM, YYYY-MM-DD, or "current".
    pub month: String,
    /// Positive currency amount to move.
    pub amount: CurrencyAmount,
    /// Category to take from. Omit to take from Ready to Assign.
    #[serde(default)]
    pub from_category_id: Option<String>,
    /// Category to give to. Omit to return money to Ready to Assign.
    #[serde(default)]
    pub to_category_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduledInput {
    pub account_id: String,
    /// Future date, YYYY-MM-DD, at most 5 years ahead.
    pub date: String,
    /// Currency units. Negative for an expense.
    pub amount: CurrencyAmount,
    pub frequency: Frequency,
    #[serde(default)]
    pub payee_id: Option<String>,
    #[serde(default)]
    pub payee_name: Option<String>,
    #[serde(default)]
    pub category_id: Option<String>,
    #[serde(default)]
    pub memo: Option<String>,
    #[serde(default)]
    pub flag_color: Option<FlagColor>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateScheduledArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    pub scheduled_transaction: ScheduledInput,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateScheduledArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    pub scheduled_transaction_id: String,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub amount: Option<CurrencyAmount>,
    #[serde(default)]
    pub frequency: Option<Frequency>,
    #[serde(default)]
    pub payee_id: Option<String>,
    #[serde(default)]
    pub payee_name: Option<String>,
    #[serde(default)]
    pub category_id: Option<String>,
    #[serde(default)]
    pub memo: Option<String>,
    #[serde(default)]
    pub flag_color: Option<FlagColor>,
    #[serde(default)]
    pub clear_flag: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteScheduledArgs {
    /// Plan id from list_plans. Omit to use YNAB_PLAN_ID. "last-used" and "default" are allowed.
    #[serde(default)]
    pub plan_id: Option<String>,
    pub scheduled_transaction_id: String,
}
