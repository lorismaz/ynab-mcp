//! Currency amounts are accepted and returned in major units (euros, dollars).
//! YNAB stores milliunits: 1 currency unit = 1000 milliunits.

use serde::Deserialize;

/// An amount in currency units, accepted as a JSON number or a decimal string.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum CurrencyAmount {
    /// Numeric currency units, for example `-12.5`.
    Number(f64),
    /// Decimal text, for example `"-12.50"` or `"12,50"`.
    Text(String),
}

impl CurrencyAmount {
    pub fn to_milliunits(&self) -> Result<i64, String> {
        match self {
            Self::Number(amount) => f64_to_milliunits(*amount),
            Self::Text(amount) => parse_decimal_str(amount),
        }
    }
}

pub fn milliunits_to_currency(milliunits: i64) -> f64 {
    (milliunits as f64) / 1000.0
}

pub fn f64_to_milliunits(amount: f64) -> Result<i64, String> {
    if !amount.is_finite() {
        return Err("amount must be a finite number".into());
    }
    if amount.abs() > 1_000_000_000.0 {
        return Err("amount is too large".into());
    }
    // Fixed 3 decimal places avoids binary float drift (10.10 -> 10100 milliunits).
    parse_decimal_str(&format!("{amount:.3}"))
}

/// Parse a decimal amount into milliunits.
///
/// A dot is the decimal separator. A comma is a decimal separator when 1 or 2
/// digits follow it (`12,50`). `1.234,56` and `1,234.56` are both accepted.
/// `12,500` is rejected so thousands separators are not guessed.
pub fn parse_decimal_str(raw: &str) -> Result<i64, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("amount is empty".into());
    }
    let (negative, unsigned) = if let Some(rest) = trimmed.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        (false, rest)
    } else {
        (false, trimmed)
    };
    if unsigned.is_empty() {
        return Err(format!("invalid amount '{raw}'"));
    }
    let normalized = normalize_number(unsigned)?;
    let (whole, frac) = match normalized.split_once('.') {
        Some((whole, frac)) => (whole, frac),
        None => (normalized.as_str(), ""),
    };
    if whole.is_empty() || !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("invalid amount '{raw}'"));
    }
    if !frac.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("invalid amount '{raw}'"));
    }
    if frac.len() > 6 {
        return Err("amount has too many decimal places".into());
    }
    let milli_digits = if frac.len() >= 3 {
        frac[..3].to_string()
    } else {
        format!("{frac:0<3}")
    };
    let round_up = frac.len() > 3 && frac.as_bytes()[3] >= b'5';
    let whole_n: i64 = whole
        .parse()
        .map_err(|_| format!("invalid amount '{raw}'"))?;
    let frac_n: i64 = milli_digits
        .parse()
        .map_err(|_| format!("invalid amount '{raw}'"))?;
    let mut milli = whole_n
        .checked_mul(1000)
        .and_then(|value| value.checked_add(frac_n))
        .ok_or_else(|| "amount is too large".to_string())?;
    if round_up {
        milli = milli
            .checked_add(1)
            .ok_or_else(|| "amount is too large".to_string())?;
    }
    if milli.unsigned_abs() > 1_000_000_000_000 {
        return Err("amount is too large".into());
    }
    if negative {
        milli = milli
            .checked_neg()
            .ok_or_else(|| "amount is too large".to_string())?;
    }
    Ok(milli)
}

fn normalize_number(input: &str) -> Result<String, String> {
    if input.chars().any(|ch| ch.is_whitespace()) {
        return Err("amount must not contain spaces; use -12.50 or 12,50".into());
    }
    let dots = input.matches('.').count();
    let commas = input.matches(',').count();
    if dots > 0 && commas > 0 {
        let last_dot = input.rfind('.').unwrap_or(0);
        let last_comma = input.rfind(',').unwrap_or(0);
        if last_comma > last_dot {
            return Ok(input.replace('.', "").replace(',', "."));
        }
        return Ok(input.replace(',', ""));
    }
    if commas > 0 {
        if commas != 1 {
            return Err(
                "amount with multiple commas is ambiguous; use a dot decimal such as 1234.56"
                    .into(),
            );
        }
        let (_, frac) = input.split_once(',').unwrap_or(("", ""));
        if (1..=2).contains(&frac.len()) && frac.bytes().all(|byte| byte.is_ascii_digit()) {
            return Ok(input.replace(',', "."));
        }
        return Err(
            "comma amounts need 1 or 2 decimal digits (12,5 or 12,50); otherwise use a dot".into(),
        );
    }
    if dots > 1 {
        return Err("amount has more than one decimal point".into());
    }
    Ok(input.to_string())
}

#[cfg(test)]
mod tests {
    use super::{f64_to_milliunits, milliunits_to_currency, parse_decimal_str};

    #[test]
    fn currency_round_trip_common_amounts() {
        let cases = [
            ("-12.50", -12_500),
            ("-12.5", -12_500),
            ("0.10", 100),
            ("10.10", 10_100),
            ("0", 0),
            ("12,50", 12_500),
            ("12,5", 12_500),
            ("1,234.56", 1_234_560),
            ("1.234,56", 1_234_560),
            ("-0.001", -1),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_decimal_str(input).unwrap(), expected, "{input}");
        }
    }

    #[test]
    fn float_conversion_avoids_binary_drift() {
        assert_eq!(f64_to_milliunits(10.10).unwrap(), 10_100);
        assert_eq!(f64_to_milliunits(-12.5).unwrap(), -12_500);
        assert_eq!(f64_to_milliunits(0.1).unwrap(), 100);
    }

    #[test]
    fn milliunits_become_currency_units() {
        assert_eq!(milliunits_to_currency(-12_500), -12.5);
        assert_eq!(milliunits_to_currency(10_100), 10.1);
    }

    #[test]
    fn ambiguous_comma_is_rejected() {
        assert!(parse_decimal_str("12,500").is_err());
    }
}
