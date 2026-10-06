//! Turn YNAB milliunit fields into currency units before they reach the model.
//! Prefer sibling `*_currency` values when the API sends them.

use serde_json::{Map, Value};

use crate::money::milliunits_to_currency;

const MONEY_FIELDS: &[&str] = &[
    "amount",
    "balance",
    "cleared_balance",
    "uncleared_balance",
    "budgeted",
    "activity",
    "income",
    "to_be_budgeted",
    "goal_target",
    "goal_under_funded",
    "goal_overall_funded",
    "goal_overall_left",
];

pub fn present_money(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                present_money(item);
            }
        }
        Value::Object(object) => {
            rewrite_object(object);
            for child in object.values_mut() {
                present_money(child);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn rewrite_object(object: &mut Map<String, Value>) {
    let keys: Vec<String> = object.keys().cloned().collect();
    for key in keys {
        if MONEY_FIELDS.contains(&key.as_str()) {
            convert_money_field(object, &key);
        }
    }
    if object.contains_key("budgeted")
        && object.contains_key("activity")
        && object.contains_key("balance")
    {
        copy_alias(object, "balance", "available");
        copy_alias(object, "balance_formatted", "available_formatted");
        copy_alias(object, "budgeted", "assigned");
        copy_alias(object, "budgeted_formatted", "assigned_formatted");
    }
    if object.contains_key("to_be_budgeted") {
        copy_alias(object, "to_be_budgeted", "ready_to_assign");
        copy_alias(
            object,
            "to_be_budgeted_formatted",
            "ready_to_assign_formatted",
        );
    }
}

fn convert_money_field(object: &mut Map<String, Value>, key: &str) {
    let Some(current) = object.get(key).cloned() else {
        return;
    };
    let currency_key = format!("{key}_currency");
    if let Some(currency) = object.get(&currency_key) {
        if currency.is_number() {
            object.insert(key.to_string(), currency.clone());
            return;
        }
    }
    if let Some(milliunits) = current.as_i64() {
        object.insert(
            key.to_string(),
            serde_json::json!(milliunits_to_currency(milliunits)),
        );
    }
}

fn copy_alias(object: &mut Map<String, Value>, from: &str, to: &str) {
    if object.contains_key(to) {
        return;
    }
    if let Some(value) = object.get(from).cloned() {
        object.insert(to.to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::present_money;

    #[test]
    fn prefers_currency_fields_and_adds_available() {
        let mut value = json!({
            "amount": -12500,
            "amount_currency": -12.5,
            "amount_formatted": "-€12.50",
            "categories": [{
                "name": "Groceries",
                "budgeted": 200000,
                "activity": -50000,
                "balance": 150000,
                "budgeted_currency": 200.0,
                "activity_currency": -50.0,
                "balance_currency": 150.0
            }],
            "to_be_budgeted": 25000
        });
        present_money(&mut value);
        assert_eq!(value["amount"], json!(-12.5));
        assert_eq!(value["categories"][0]["available"], json!(150.0));
        assert_eq!(value["categories"][0]["assigned"], json!(200.0));
        assert_eq!(value["ready_to_assign"], json!(25.0));
    }

    #[test]
    fn divides_milliunits_when_currency_field_is_absent() {
        let mut value = json!({"amount": -12500});
        present_money(&mut value);
        assert_eq!(value["amount"], json!(-12.5));
    }
}
