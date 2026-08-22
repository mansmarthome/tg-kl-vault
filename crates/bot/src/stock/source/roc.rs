//! ROC (Minguo) date and Taiwan-style number parsing for the TWSE/TPEx official
//! dumps. **Pure** — no network, no DB.

/// Converts a ROC date to ISO `YYYY-MM-DD`. ROC year = Gregorian − 1911, so
/// `"1150821"` → `"2026-08-21"`. Accepts both packed (`1150821`) and
/// slash-separated (`114/08/21`) forms. The last four digits are MMDD; the rest
/// is the ROC year. Returns `None` for anything it can't confidently parse —
/// the fallback's date gate treats an unparseable date as "refuse", which is
/// the safe default.
pub fn roc_to_iso(raw: &str) -> Option<String> {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 5 {
        return None;
    }
    let (year_part, mmdd) = digits.split_at(digits.len() - 4);
    let roc_year: i32 = year_part.parse().ok()?;
    let month: u32 = mmdd[..2].parse().ok()?;
    let day: u32 = mmdd[2..].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || roc_year < 1 {
        return None;
    }
    Some(format!("{:04}-{:02}-{:02}", roc_year + 1911, month, day))
}

/// Parses a TWSE/TPEx numeric field to `Option<f64>`. Strips thousands commas
/// (only the legacy `STOCK_DAY` endpoint uses them, but stripping is harmless)
/// and trailing whitespace (TPEx `Change` ships as `"-52.00 "`). Returns `None`
/// for the no-trade sentinels (`""`, `"--"`) and for non-numeric text — TPEx
/// `Change` can be a Chinese word like `"除息"`, which must become `None`, not
/// `0.0`.
pub fn parse_tw_number(raw: &str) -> Option<f64> {
    let cleaned = raw.trim().replace(',', "");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '-') {
        return None;
    }
    cleaned.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roc_dates_convert_to_iso() {
        assert_eq!(roc_to_iso("1150821").as_deref(), Some("2026-08-21"));
        assert_eq!(roc_to_iso("1140101").as_deref(), Some("2025-01-01"));
        assert_eq!(roc_to_iso("114/08/21").as_deref(), Some("2025-08-21"));
        // Two-digit ROC year (pre-2011).
        assert_eq!(roc_to_iso("990228").as_deref(), Some("2010-02-28"));
    }

    #[test]
    fn bad_roc_dates_are_none() {
        assert_eq!(roc_to_iso(""), None);
        assert_eq!(roc_to_iso("abc"), None);
        assert_eq!(roc_to_iso("1151321"), None); // month 13
        assert_eq!(roc_to_iso("1150832"), None); // day 32
    }

    #[test]
    fn tw_number_treats_double_dash_and_empty_as_no_trade() {
        assert_eq!(parse_tw_number(""), None);
        assert_eq!(parse_tw_number("--"), None);
        assert_eq!(parse_tw_number("---"), None);
    }

    #[test]
    fn tpex_change_may_be_chinese_text_and_is_optional() {
        assert_eq!(parse_tw_number("除息 "), None);
        assert_eq!(parse_tw_number("除權息"), None);
    }

    #[test]
    fn numbers_strip_commas_and_trailing_space() {
        assert_eq!(parse_tw_number("2,410.00"), Some(2410.0));
        assert_eq!(parse_tw_number("-52.00 "), Some(-52.0));
        assert_eq!(parse_tw_number("+10.00"), Some(10.0));
        assert_eq!(parse_tw_number(" 17158844 "), Some(17_158_844.0));
    }
}
