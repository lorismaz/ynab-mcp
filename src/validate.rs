use chrono::{NaiveDate, Utc};

/// Ids are interpolated into URL paths. Allow YNAB uuids plus `last-used` and `default`.
pub fn validate_path_id(label: &str, value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 80 {
        return Err(format!("{label} must be 1-80 characters"));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(format!(
            "{label} may only contain letters, numbers, hyphens, and underscores"
        ));
    }
    Ok(())
}

pub fn normalize_date(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .map(|date| date.format("%Y-%m-%d").to_string())
        .map_err(|_| format!("date must be YYYY-MM-DD, got '{trimmed}'"))
}

/// Accept `current`, `YYYY-MM`, or `YYYY-MM-DD` (normalized to the first of that month).
pub fn normalize_month(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("current") {
        return Ok("current".into());
    }
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Ok(format!("{}-01", date.format("%Y-%m")));
    }
    if NaiveDate::parse_from_str(&format!("{trimmed}-01"), "%Y-%m-%d").is_ok() {
        return Ok(format!("{trimmed}-01"));
    }
    Err("month must be YYYY-MM, YYYY-MM-DD, or current".into())
}

pub fn today_utc() -> NaiveDate {
    Utc::now().date_naive()
}

pub fn require_future_date(date: &str) -> Result<(), String> {
    let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| format!("date must be YYYY-MM-DD, got '{date}'"))?;
    if parsed <= today_utc() {
        return Err(
            "scheduled transaction date must be after today (UTC). YNAB rejects today and past dates"
                .into(),
        );
    }
    Ok(())
}

pub fn check_len(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.chars().count() > max {
        return Err(format!("{label} must be at most {max} characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{normalize_month, validate_path_id};

    #[test]
    fn months_normalize_to_the_first() {
        assert_eq!(normalize_month("current").unwrap(), "current");
        assert_eq!(normalize_month("2026-10").unwrap(), "2026-10-01");
        assert_eq!(normalize_month("2026-10-15").unwrap(), "2026-10-01");
        assert!(normalize_month("2026-13").is_err());
    }

    #[test]
    fn path_ids_reject_traversal() {
        assert!(validate_path_id("plan_id", "last-used").is_ok());
        assert!(validate_path_id("plan_id", "../etc").is_err());
        assert!(validate_path_id("plan_id", "a/b").is_err());
    }
}
