use crate::Error;
use chrono::NaiveDate;

/// Explicit numeric dates. The separator disambiguates the field order
/// (spec: Date Parsing): `-` = ISO YMD, `.` = European DMY, `/` = US MDY.
/// Returns None when the input isn't shaped like a numeric date at all
/// (caller falls through to natural-language parsing).
pub fn parse_explicit(input: &str) -> Option<Result<NaiveDate, Error>> {
    let fmt = if input.contains('-') {
        "%Y-%m-%d"
    } else if input.contains('.') {
        "%d.%m.%Y"
    } else if input.contains('/') {
        "%m/%d/%Y"
    } else {
        return None;
    };
    // Only treat it as numeric-explicit if it's digits + separators;
    // "next friday" contains no separator and never reaches here, but
    // guard against inputs like "3/4 done" leaking in.
    if !input.chars().all(|c| c.is_ascii_digit() || "-./".contains(c)) {
        return None;
    }
    Some(
        NaiveDate::parse_from_str(input, fmt)
            .map_err(|_| Error::DateParse(input.to_string())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn iso_dash_is_year_month_day() {
        assert_eq!(parse_explicit("2024-03-05").unwrap().unwrap(), d(2024, 3, 5));
    }

    #[test]
    fn dot_is_day_month_year() {
        assert_eq!(parse_explicit("05.12.2024").unwrap().unwrap(), d(2024, 12, 5));
    }

    #[test]
    fn slash_is_month_day_year() {
        assert_eq!(parse_explicit("11/27/2024").unwrap().unwrap(), d(2024, 11, 27));
    }

    #[test]
    fn impossible_calendar_date_is_an_error() {
        assert!(parse_explicit("31.02.2024").unwrap().is_err());
        assert!(parse_explicit("2024-13-01").unwrap().is_err());
        assert!(parse_explicit("02/30/2024").unwrap().is_err());
    }

    #[test]
    fn non_numeric_input_is_none() {
        assert!(parse_explicit("next friday").is_none());
        assert!(parse_explicit("Feb 3").is_none());
        assert!(parse_explicit("today").is_none());
    }
}
