use crate::Error;
use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone};

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

/// Parse any accepted date input to a calendar date. Explicit numeric
/// formats first (separator-disambiguated), then English natural language
/// via `interim`. `now` anchors relative expressions and is injected so
/// tests never depend on the wall clock.
pub fn parse_date(input: &str, now: DateTime<FixedOffset>) -> Result<NaiveDate, Error> {
    if let Some(explicit) = parse_explicit(input) {
        return explicit;
    }
    // interim's grammar has no "in N units" form ("in" is an unsupported
    // token); it reads bare "N units" as the future shift the spec means.
    // Normalize the spec-required "in 5 days" shape before delegating.
    let normalized = input
        .strip_prefix("in ")
        .or_else(|| input.strip_prefix("In "))
        .unwrap_or(input);
    interim::parse_date_string(normalized, now, interim::Dialect::Us)
        .map(|dt| dt.date_naive())
        .map_err(|_| Error::DateParse(input.to_string()))
}

/// 23:59:59 on `date` in timezone `tz`, as a fixed-offset instant
/// (spec: Timezone Handling). On a DST gap/ambiguity, take the earliest
/// valid interpretation.
pub fn end_of_day<Tz: TimeZone>(date: NaiveDate, tz: &Tz) -> Result<DateTime<FixedOffset>, Error> {
    let naive = date
        .and_hms_opt(23, 59, 59)
        .expect("23:59:59 is always a valid time");
    tz.from_local_datetime(&naive)
        .earliest()
        .map(|dt| dt.fixed_offset())
        .ok_or_else(|| Error::DateParse(date.to_string()))
}

/// The full `--due` / date-filter pipeline: parse, then resolve to end of
/// day in `tz`. The CLI passes `Local::now().fixed_offset()` and `&Local`.
pub fn parse_to_instant<Tz: TimeZone>(
    input: &str,
    now: DateTime<FixedOffset>,
    tz: &Tz,
) -> Result<DateTime<FixedOffset>, Error> {
    end_of_day(parse_date(input, now)?, tz)
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
        assert!(parse_explicit("3/4 done").is_none());
        assert!(parse_explicit("mid-march").is_none());
        assert!(parse_explicit("v1.2.3x").is_none());
    }

    use chrono::{DateTime, FixedOffset};

    fn now() -> DateTime<FixedOffset> {
        // Wednesday 2024-03-13, 10:00 +01:00
        DateTime::parse_from_rfc3339("2024-03-13T10:00:00+01:00").unwrap()
    }

    #[test]
    fn natural_relative_words() {
        assert_eq!(parse_date("today", now()).unwrap(), d(2024, 3, 13));
        assert_eq!(parse_date("tomorrow", now()).unwrap(), d(2024, 3, 14));
        assert_eq!(parse_date("yesterday", now()).unwrap(), d(2024, 3, 12));
    }

    #[test]
    fn natural_shifted() {
        assert_eq!(parse_date("in 5 days", now()).unwrap(), d(2024, 3, 18));
        assert_eq!(parse_date("3 weeks ago", now()).unwrap(), d(2024, 2, 21));
        assert_eq!(parse_date("next friday", now()).unwrap(), d(2024, 3, 15));
        assert_eq!(parse_date("last tuesday", now()).unwrap(), d(2024, 3, 12));
    }

    #[test]
    fn natural_month_day() {
        assert_eq!(parse_date("Feb 3", now()).unwrap(), d(2024, 2, 3));
        assert_eq!(parse_date("March 15", now()).unwrap(), d(2024, 3, 15));
    }

    #[test]
    fn explicit_formats_win_over_natural() {
        assert_eq!(parse_date("05.12.2024", now()).unwrap(), d(2024, 12, 5));
    }

    #[test]
    fn garbage_is_date_parse_error() {
        let err = parse_date("not a date at all", now()).unwrap_err();
        assert_eq!(err.to_string(), "cannot parse date: \"not a date at all\"");
    }

    #[test]
    fn end_of_day_carries_the_given_offset() {
        let tz = FixedOffset::east_opt(3600).unwrap(); // +01:00
        let instant = end_of_day(d(2024, 3, 15), &tz).unwrap();
        assert_eq!(instant.to_rfc3339(), "2024-03-15T23:59:59+01:00");
    }

    #[test]
    fn parse_to_instant_is_end_of_day_of_parsed_date() {
        let tz = FixedOffset::east_opt(-5 * 3600).unwrap(); // -05:00
        let instant = parse_to_instant("tomorrow", now(), &tz).unwrap();
        assert_eq!(instant.to_rfc3339(), "2024-03-14T23:59:59-05:00");
    }
}
