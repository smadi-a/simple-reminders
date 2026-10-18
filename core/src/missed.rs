//! Computing which reminders were due while the user was logged out (login sweep).

use crate::model::Reminder;
use crate::recurrence::due_instants_between;
use chrono::NaiveDateTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissedOccurrence {
    pub reminder_id: i64,
    pub due_instant: NaiveDateTime,
}

/// For each reminder, finds the latest instant (if any) it was due strictly after
/// `window_start_exclusive` and up to `now`. Multiple occurrences inside that window (e.g.
/// a daily reminder missed over a multi-day outage) collapse to a single entry — the
/// latest one — rather than one row per missed instance, matching the app's "no history"
/// design: the point is just to surface that it was missed and let the user clear it once.
pub fn compute_missed(
    reminders: &[Reminder],
    window_start_exclusive: NaiveDateTime,
    now: NaiveDateTime,
) -> Vec<MissedOccurrence> {
    reminders
        .iter()
        .filter_map(|reminder| {
            due_instants_between(
                &reminder.recurrence,
                reminder.start_date,
                reminder.time_of_day,
                window_start_exclusive,
                now,
            )
            .last()
            .map(|&due_instant| MissedOccurrence { reminder_id: reminder.id, due_instant })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RecurrenceRule;
    use chrono::{NaiveDate, NaiveTime};

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn time(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        date(y, mo, d).and_time(time(h, mi))
    }

    fn reminder(id: i64, recurrence: RecurrenceRule, start_date: NaiveDate, time_of_day: NaiveTime) -> Reminder {
        Reminder {
            id,
            title: format!("reminder-{id}"),
            time_of_day,
            recurrence,
            start_date,
            last_fired_due_instant: None,
            missed: false,
            missed_due_instant: None,
            snoozed_until: None,
            created_at: start_date.and_time(time_of_day),
        }
    }

    #[test]
    fn compute_missed_collapses_multiple_daily_occurrences_to_latest() {
        let r = reminder(1, RecurrenceRule::Daily, date(2026, 1, 1), time(9, 0));
        let window_start = at(2026, 3, 10, 0, 0);
        let now = at(2026, 3, 13, 12, 0);
        let missed = compute_missed(&[r], window_start, now);
        // now = 2026-03-13 12:00, so that day's 9:00 occurrence is also within the window.
        assert_eq!(
            missed,
            vec![MissedOccurrence { reminder_id: 1, due_instant: at(2026, 3, 13, 9, 0) }]
        );
    }

    #[test]
    fn compute_missed_returns_none_when_nothing_missed() {
        let r = reminder(1, RecurrenceRule::Daily, date(2026, 1, 1), time(9, 0));
        let window_start = at(2026, 3, 10, 9, 30);
        let now = at(2026, 3, 10, 18, 0);
        assert_eq!(compute_missed(&[r], window_start, now), Vec::new());
    }

    #[test]
    fn compute_missed_one_time_reminder_past_due_while_logged_out() {
        let r = reminder(1, RecurrenceRule::OneTime, date(2026, 3, 11), time(9, 0));
        let window_start = at(2026, 3, 10, 0, 0);
        let now = at(2026, 3, 13, 0, 0);
        assert_eq!(
            compute_missed(&[r], window_start, now),
            vec![MissedOccurrence { reminder_id: 1, due_instant: at(2026, 3, 11, 9, 0) }]
        );
    }
}
