use crate::model::Status;
use crate::store::TaskRecord;
use chrono::{DateTime, Days, FixedOffset, NaiveDate, TimeZone};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFilter {
    #[default]
    Open,
    Done,
    All,
    Overdue,
    Urgent,
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub status: StatusFilter,
    /// AND semantics, exact-element match.
    pub tags: Vec<String>,
    /// OR semantics.
    pub sources: Vec<String>,
    /// Exact calendar-date match, read in the stored instant's own offset.
    pub due_on: Option<NaiveDate>,
    pub created_before: Option<DateTime<FixedOffset>>,
    pub created_after: Option<DateTime<FixedOffset>>,
    pub modified_before: Option<DateTime<FixedOffset>>,
    pub modified_after: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Due,
    Created,
    Modified,
    Title,
}

/// Calendar date of `instant`, taken from the instant's own baked-in
/// offset — the offset that was local at the moment it was saved — never
/// reprojected through a different instant's offset (e.g. "now"'s). This
/// avoids DST reprojection errors: reprojecting through a different
/// instant's offset when the two straddle a DST boundary silently shifts
/// the calendar date (a real bug here; don't reintroduce it). When a due
/// date was saved from a different timezone than the querying machine's
/// current one, the resulting calendar date reflects where/when it was
/// set, not the querying machine's current local date.
fn local_date(instant: DateTime<FixedOffset>) -> NaiveDate {
    instant.date_naive()
}

fn matches_status<Tz: TimeZone>(
    record: &TaskRecord,
    status: StatusFilter,
    now: DateTime<FixedOffset>,
    timezone: &Tz,
) -> bool {
    let task = &record.task;
    match status {
        StatusFilter::All => true,
        StatusFilter::Open => task.status == Status::Open,
        StatusFilter::Done => task.status == Status::Done,
        // Due today is NOT overdue: the stored instant is end of day, so
        // it isn't in the past until midnight (spec: list filters).
        StatusFilter::Overdue => task.status == Status::Open && task.due.is_some_and(|d| d < now),
        StatusFilter::Urgent => {
            let today = now.with_timezone(timezone).date_naive();
            let tomorrow = today + Days::new(1);
            task.status == Status::Open
                && task.due.is_some_and(|d| {
                    let date = d.with_timezone(timezone).date_naive();
                    date == today || date == tomorrow
                })
        }
    }
}

fn matches<Tz: TimeZone>(
    record: &TaskRecord,
    filter: &Filter,
    now: DateTime<FixedOffset>,
    timezone: &Tz,
) -> bool {
    let task = &record.task;
    matches_status(record, filter.status, now, timezone)
        && filter.tags.iter().all(|t| task.tags.contains(t))
        && (filter.sources.is_empty() || filter.sources.iter().any(|s| task.source.contains(s)))
        && filter
            .due_on
            .is_none_or(|d| task.due.is_some_and(|due| local_date(due) == d))
        && filter.created_before.is_none_or(|b| task.created < b)
        && filter.created_after.is_none_or(|a| task.created > a)
        && filter.modified_before.is_none_or(|b| task.modified < b)
        && filter.modified_after.is_none_or(|a| task.modified > a)
}

/// Filter then sort, entirely in memory (spec: Query Engine).
pub fn apply<Tz: TimeZone>(
    records: Vec<TaskRecord>,
    filter: &Filter,
    sort: SortKey,
    now: DateTime<FixedOffset>,
    timezone: &Tz,
) -> Vec<TaskRecord> {
    let mut out: Vec<TaskRecord> = records
        .into_iter()
        .filter(|r| matches(r, filter, now, timezone))
        .collect();
    match sort {
        // Due ascending, no-due last, created-ascending tiebreak (spec default).
        SortKey::Due => out.sort_by_key(|r| (r.task.due.is_none(), r.task.due, r.task.created)),
        SortKey::Created => out.sort_by_key(|r| r.task.created),
        SortKey::Modified => out.sort_by_key(|r| r.task.modified),
        SortKey::Title => out.sort_by(|a, b| a.task.title.cmp(&b.task.title)),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Status, Task};
    use crate::store::TaskRecord;
    use chrono::DateTime;

    fn dt(s: &str) -> chrono::DateTime<chrono::FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    // "now" for all tests: 2024-03-13 10:00 +01:00 (a Wednesday)
    fn now() -> chrono::DateTime<chrono::FixedOffset> {
        dt("2024-03-13T10:00:00+01:00")
    }

    fn apply(
        records: Vec<TaskRecord>,
        filter: &Filter,
        sort: SortKey,
        now: DateTime<FixedOffset>,
    ) -> Vec<TaskRecord> {
        super::apply(
            records,
            filter,
            sort,
            now,
            &FixedOffset::east_opt(3600).unwrap(),
        )
    }

    fn rec(
        id: &str,
        status: Status,
        due: Option<&str>,
        tags: &[&str],
        source: &[&str],
    ) -> TaskRecord {
        TaskRecord {
            task: Task {
                id: id.into(),
                title: format!("Task {id}"),
                status,
                due: due.map(dt),
                tags: tags.iter().map(|s| s.to_string()).collect(),
                source: source.iter().map(|s| s.to_string()).collect(),
                created: dt("2024-01-01T09:00:00+01:00"),
                modified: dt("2024-02-01T09:00:00+01:00"),
                related: vec![],
                description: None,
                tasks: vec![],
            },
            project: "proj".into(),
            path: std::path::PathBuf::from(format!("/x/{id}.toml")),
        }
    }

    fn ids(records: &[TaskRecord]) -> Vec<&str> {
        records.iter().map(|r| r.task.id.as_str()).collect()
    }

    fn fixture() -> Vec<TaskRecord> {
        vec![
            // open, overdue (due yesterday)
            rec(
                "overdue1",
                Status::Open,
                Some("2024-03-12T23:59:59+01:00"),
                &["okr"],
                &["meet-a"],
            ),
            // open, due today → urgent, NOT overdue
            rec(
                "duetoday",
                Status::Open,
                Some("2024-03-13T23:59:59+01:00"),
                &["okr", "review"],
                &[],
            ),
            // open, due tomorrow → urgent
            rec(
                "duetomor",
                Status::Open,
                Some("2024-03-14T23:59:59+01:00"),
                &[],
                &["meet-b"],
            ),
            // open, due far future
            rec(
                "farfutur",
                Status::Open,
                Some("2024-06-01T23:59:59+02:00"),
                &["review"],
                &[],
            ),
            // open, no due date
            rec("nodueyet", Status::Open, None, &["okr"], &["meet-a"]),
            // done
            rec(
                "done0001",
                Status::Done,
                Some("2024-03-01T23:59:59+01:00"),
                &["okr"],
                &[],
            ),
        ]
    }

    #[test]
    fn default_filter_keeps_open_only() {
        let out = apply(fixture(), &Filter::default(), SortKey::Title, now());
        assert!(!ids(&out).contains(&"done0001"));
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn status_done_and_all() {
        let done = Filter {
            status: StatusFilter::Done,
            ..Filter::default()
        };
        assert_eq!(
            ids(&apply(fixture(), &done, SortKey::Title, now())),
            vec!["done0001"]
        );
        let all = Filter {
            status: StatusFilter::All,
            ..Filter::default()
        };
        assert_eq!(apply(fixture(), &all, SortKey::Title, now()).len(), 6);
    }

    #[test]
    fn overdue_excludes_due_today() {
        let f = Filter {
            status: StatusFilter::Overdue,
            ..Filter::default()
        };
        assert_eq!(
            ids(&apply(fixture(), &f, SortKey::Title, now())),
            vec!["overdue1"]
        );
    }

    #[test]
    fn urgent_is_today_or_tomorrow_open_only() {
        let f = Filter {
            status: StatusFilter::Urgent,
            ..Filter::default()
        };
        assert_eq!(
            ids(&apply(fixture(), &f, SortKey::Title, now())),
            vec!["duetoday", "duetomor"]
        );
    }

    #[test]
    fn urgent_uses_the_querying_timezone_not_the_stored_due_offset() {
        let now = dt("2024-03-13T23:30:00+00:00");
        let due = rec(
            "nearby01",
            Status::Open,
            Some("2024-03-15T00:30:00+14:00"),
            &[],
            &[],
        );
        let filter = Filter {
            status: StatusFilter::Urgent,
            ..Filter::default()
        };
        assert_eq!(
            ids(&super::apply(
                vec![due],
                &filter,
                SortKey::Title,
                now,
                &FixedOffset::east_opt(0).unwrap(),
            )),
            vec!["nearby01"]
        );
    }

    #[test]
    fn urgent_uses_the_querying_timezone_when_stored_offset_is_behind() {
        let now = dt("2024-03-13T23:30:00+00:00");
        let due = rec(
            "nearby02",
            Status::Open,
            Some("2024-03-14T23:30:00-10:00"),
            &[],
            &[],
        );
        let filter = Filter {
            status: StatusFilter::Urgent,
            ..Filter::default()
        };
        assert_eq!(
            ids(&super::apply(
                vec![due],
                &filter,
                SortKey::Title,
                now,
                &FixedOffset::east_opt(14 * 3600).unwrap(),
            )),
            vec!["nearby02"]
        );
    }

    #[test]
    fn tag_filter_is_and_with_exact_elements() {
        let f = Filter {
            tags: vec!["okr".into(), "review".into()],
            ..Filter::default()
        };
        assert_eq!(
            ids(&apply(fixture(), &f, SortKey::Title, now())),
            vec!["duetoday"]
        );
        // exact element: "ok" must not match "okr"
        let f2 = Filter {
            tags: vec!["ok".into()],
            ..Filter::default()
        };
        assert!(apply(fixture(), &f2, SortKey::Title, now()).is_empty());
    }

    #[test]
    fn source_filter_is_or() {
        let f = Filter {
            sources: vec!["meet-a".into(), "meet-b".into()],
            ..Filter::default()
        };
        assert_eq!(
            ids(&apply(fixture(), &f, SortKey::Title, now())),
            vec!["duetomor", "nodueyet", "overdue1"]
        );
    }

    #[test]
    fn due_on_matches_calendar_date_in_stored_offset() {
        let f = Filter {
            due_on: chrono::NaiveDate::from_ymd_opt(2024, 3, 13),
            status: StatusFilter::All,
            ..Filter::default()
        };
        assert_eq!(
            ids(&apply(fixture(), &f, SortKey::Title, now())),
            vec!["duetoday"]
        );
    }

    #[test]
    fn due_calendar_date_is_stable_across_dst_offsets() {
        // Winter due (+01:00) queried from a summer `now` (+02:00): the
        // calendar date must come from the stored instant's own offset,
        // never reprojected through `now`'s fixed offset (which would
        // shift it to Jan 16).
        let winter_due = rec(
            "dstguard1",
            Status::Open,
            Some("2024-01-15T23:59:59+01:00"),
            &[],
            &[],
        );
        let summer_now = dt("2024-07-01T12:00:00+02:00");
        let f = Filter {
            due_on: chrono::NaiveDate::from_ymd_opt(2024, 1, 15),
            status: StatusFilter::All,
            ..Filter::default()
        };
        assert_eq!(
            ids(&apply(vec![winter_due], &f, SortKey::Title, summer_now)),
            vec!["dstguard1"]
        );
    }

    #[test]
    fn created_and_modified_range_filters() {
        let f = Filter {
            created_before: Some(dt("2024-01-02T00:00:00+01:00")),
            ..Filter::default()
        };
        assert_eq!(apply(fixture(), &f, SortKey::Title, now()).len(), 5); // all open created 2024-01-01
        let none = Filter {
            created_after: Some(dt("2024-01-02T00:00:00+01:00")),
            ..Filter::default()
        };
        assert!(apply(fixture(), &none, SortKey::Title, now()).is_empty());
        let modified = Filter {
            modified_after: Some(dt("2024-01-15T00:00:00+01:00")),
            modified_before: Some(dt("2024-02-15T00:00:00+01:00")),
            ..Filter::default()
        };
        assert_eq!(apply(fixture(), &modified, SortKey::Title, now()).len(), 5);
    }

    #[test]
    fn default_sort_due_ascending_no_due_last() {
        let out = apply(fixture(), &Filter::default(), SortKey::Due, now());
        assert_eq!(
            ids(&out),
            vec!["overdue1", "duetoday", "duetomor", "farfutur", "nodueyet"]
        );
    }

    #[test]
    fn sort_by_title_created_modified() {
        let by_title = apply(fixture(), &Filter::default(), SortKey::Title, now());
        assert_eq!(
            ids(&by_title),
            vec!["duetoday", "duetomor", "farfutur", "nodueyet", "overdue1"] // "Task <id>" lexical
        );
        // created/modified are equal across fixture → stable order = input order
        let by_created = apply(fixture(), &Filter::default(), SortKey::Created, now());
        assert_eq!(by_created.len(), 5);
    }
}
