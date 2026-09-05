use crate::models::RawRow;
use chrono::{Duration, NaiveDateTime};
use std::collections::HashSet;

const TIME_FMT: &str = "%Y/%m/%d %H:%M:%S";
const MAX_ACTIVITY_AGE_SECONDS: i64 = 600;

fn parse_time(s: &str) -> Option<NaiveDateTime> {
    if s.is_empty() {
        return None;
    }
    NaiveDateTime::parse_from_str(s, TIME_FMT).ok()
}

pub(crate) fn qualified(activity: &RawRow, seen: &HashSet<String>, now: NaiveDateTime) -> bool {
    if seen.contains(&activity.id) {
        return false;
    }
    if !activity.checkin_status.is_empty()
        || !activity.checkin_date.is_empty()
        || !activity.student_id.is_empty()
    {
        return false;
    }
    if activity.status_id == "1" || activity.status_name == "出勤" {
        return false;
    }
    if activity.can_apply != "1" {
        return false;
    }
    if let Some(limit) = parse_time(&activity.apply_limit)
        && now > limit
    {
        return false;
    }
    if let Some(created) = parse_time(&activity.creater_at)
        && now - created > Duration::seconds(MAX_ACTIVITY_AGE_SECONDS)
    {
        return false;
    }
    true
}

pub(crate) fn qualified_activities<'a>(
    rows: &'a [RawRow],
    seen: &HashSet<String>,
    now: NaiveDateTime,
) -> Vec<&'a RawRow> {
    rows.iter()
        .filter(|activity| qualified(activity, seen, now))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RawRow;

    fn row(json: &str) -> RawRow {
        serde_json::from_str(json).expect("Json should be parsed")
    }

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, TIME_FMT).unwrap()
    }

    fn empty_seen() -> HashSet<String> {
        HashSet::new()
    }

    const NOW: &str = "2026/08/10 10:00:00";

    #[test]
    fn fresh_activity_is_qualified() {
        let activity = row(r#"{"ID":"1","CanApply":"1"}"#);
        assert!(qualified(&activity, &empty_seen(), dt(NOW)));
    }

    #[test]
    fn seen_activities_are_skipped() {
        let activity = row(r#"{"ID":"1","CanApply":"1"}"#);
        let mut seen = HashSet::new();
        seen.insert("1".to_string());
        assert!(!qualified(&activity, &seen, dt(NOW)));
    }

    #[test]
    fn signed_activities_are_skipped() {
        for json in [
            r#"{"ID":"1","CheckInStatus":"ok"}"#,
            r#"{"ID":"1","CheckInDate":"2026/08/10 09:00:00"}"#,
            r#"{"ID":"1","StudentID":"2023001"}"#,
        ] {
            assert!(!qualified(&row(json), &empty_seen(), dt(NOW)));
        }
    }

    #[test]
    fn attended_activities_are_skipped() {
        assert!(!qualified(
            &row(r#"{"ID":"1","StatusID":"1"}"#),
            &empty_seen(),
            dt(NOW)
        ));
        assert!(!qualified(
            &row(r#"{"ID":"1","StatusName":"出勤"}"#),
            &empty_seen(),
            dt(NOW)
        ));
    }

    #[test]
    fn cannot_apply_activities_are_skipped() {
        assert!(!qualified(
            &row(r#"{"ID":"1","CanApply":"0"}"#),
            &empty_seen(),
            dt(NOW)
        ));
        assert!(!qualified(&row(r#"{"ID":"1"}"#), &empty_seen(), dt(NOW)));
    }

    #[test]
    fn expired_activities_are_skipped() {
        let activity = row(r#"{"ID":"1","CanApply":"1","ApplyLimitDate":"2026/08/10 09:59:00"}"#);
        assert!(!qualified(&activity, &empty_seen(), dt(NOW)));
    }

    #[test]
    fn too_old_activities_are_skipped() {
        let old = row(r#"{"ID":"1","CanApply":"1","CreaterDate":"2026/08/10 09:20:00"}"#);
        assert!(!qualified(&old, &empty_seen(), dt(NOW)));
        let quarter_hour = row(r#"{"ID":"1","CanApply":"1","CreaterDate":"2026/08/10 09:35:00"}"#);
        assert!(!qualified(&quarter_hour, &empty_seen(), dt(NOW)));
        let boundary = row(r#"{"ID":"1","CanApply":"1","CreaterDate":"2026/08/10 09:50:00"}"#);
        assert!(qualified(&boundary, &empty_seen(), dt(NOW)));
        let fresh = row(r#"{"ID":"1","CanApply":"1","CreaterDate":"2026/08/10 09:55:00"}"#);
        assert!(qualified(&fresh, &empty_seen(), dt(NOW)));
    }

    #[test]
    fn empty_and_bad_times_are_tolerated() {
        let activity =
            row(r#"{"ID":"1","CanApply":"1","ApplyLimitDate":"","CreaterDate":"不是时间"}"#);
        assert!(qualified(&activity, &empty_seen(), dt(NOW)));
    }

    #[test]
    fn pick_all_qualified_keeping_order() {
        let rows = vec![
            row(r#"{"ID":"1","CanApply":"1"}"#),
            row(r#"{"ID":"2","CheckInDate":"2026/08/10 09:00:00"}"#),
            row(r#"{"ID":"3","CanApply":"1"}"#),
        ];
        let picked = qualified_activities(&rows, &empty_seen(), dt(NOW));
        let ids: Vec<&str> = picked.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["1", "3"]);
    }
}
