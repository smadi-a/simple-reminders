use crate::model::RecurrenceRule;
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

/// Iteration safety valve: recurrence search should always converge within a handful of
/// steps (daily/weekly: days; monthly: months); this guards against a logic bug turning
/// into an infinite loop rather than being a real limit on how far ahead we search.
const MAX_STEPS: u32 = 1000;

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let next_month_first = NaiveDate::from_ymd_opt(next_year, next_month, 1).unwrap();
    let this_month_first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    (next_month_first - this_month_first).num_days() as u32
}

/// A "day of month" target clamped to the actual length of the given month
/// (e.g. day 31 in February becomes Feb 28, or Feb 29 in a leap year).
fn clamped_monthly_date(year: i32, month: u32, day: u8) -> NaiveDate {
    let max_day = days_in_month(year, month);
    let clamped = (day as u32).min(max_day).max(1);
    NaiveDate::from_ymd_opt(year, month, clamped).expect("clamped day is always valid")
}

fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 { (year + 1, 1) } else { (year, month + 1) }
}

/// Finds the next instant this reminder is due, strictly after `after`.
///
/// Pure and clock-free by design: callers pass in "now" (or any other reference instant)
/// explicitly, which is what makes this deterministically unit-testable and reusable for
/// both the live poller and the missed-reminders login sweep.
pub fn next_occurrence_after(
    rule: &RecurrenceRule,
    start_date: NaiveDate,
    time_of_day: NaiveTime,
    after: NaiveDateTime,
) -> Option<NaiveDateTime> {
    match rule {
        RecurrenceRule::OneTime => {
            let candidate = start_date.and_time(time_of_day);
            (candidate > after).then_some(candidate)
        }

        RecurrenceRule::Daily => {
            let mut date = start_date.max(after.date());
            for _ in 0..MAX_STEPS {
                let candidate = date.and_time(time_of_day);
                if candidate > after {
                    return Some(candidate);
                }
                date = date.succ_opt()?;
            }
            None
        }

        RecurrenceRule::Weekdays => {
            let mut date = start_date.max(after.date());
            for _ in 0..MAX_STEPS {
                if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                    let candidate = date.and_time(time_of_day);
                    if candidate > after {
                        return Some(candidate);
                    }
                }
                date = date.succ_opt()?;
            }
            None
        }

        RecurrenceRule::Weekly { days } => {
            if days.is_empty() {
                return None;
            }
            let mut date = start_date.max(after.date());
            for _ in 0..MAX_STEPS {
                if days.contains(&date.weekday()) {
                    let candidate = date.and_time(time_of_day);
                    if candidate > after {
                        return Some(candidate);
                    }
                }
                date = date.succ_opt()?;
            }
            None
        }

        RecurrenceRule::Fortnightly => {
            // Jump close to `after` first (in 14-day steps from `start_date`) rather than
            // walking one cycle at a time, so a start date from years ago doesn't exhaust
            // `MAX_STEPS` before reaching the present.
            let mut date = start_date;
            if after.date() > start_date {
                let days_since_start = (after.date() - start_date).num_days();
                let periods = days_since_start / 14;
                date = start_date + chrono::Duration::days(periods * 14);
            }
            for _ in 0..MAX_STEPS {
                let candidate = date.and_time(time_of_day);
                if candidate > after {
                    return Some(candidate);
                }
                date = date.checked_add_signed(chrono::Duration::days(14))?;
            }
            None
        }

        RecurrenceRule::MonthlyOnDay { day } => {
            let (mut year, mut month) = (start_date.year(), start_date.month());
            while (year, month) < (after.year(), after.month()) {
                let (ny, nm) = next_month(year, month);
                year = ny;
                month = nm;
            }
            for _ in 0..MAX_STEPS {
                let date = clamped_monthly_date(year, month, *day);
                if date >= start_date {
                    let candidate = date.and_time(time_of_day);
                    if candidate > after {
                        return Some(candidate);
                    }
                }
                let (ny, nm) = next_month(year, month);
                year = ny;
                month = nm;
            }
            None
        }
    }
}

/// All instants this reminder was due strictly after `window_start_exclusive` and up to
/// and including `window_end_inclusive`. Used both for the short live-poll window (ticked
/// every ~20-30s, typically yields 0-1 entries) and the much wider login/missed-reminders
/// sweep window (`settings.last_seen_at` .. now, which can span days).
pub fn due_instants_between(
    rule: &RecurrenceRule,
    start_date: NaiveDate,
    time_of_day: NaiveTime,
    window_start_exclusive: NaiveDateTime,
    window_end_inclusive: NaiveDateTime,
) -> Vec<NaiveDateTime> {
    let mut result = Vec::new();
    if window_end_inclusive <= window_start_exclusive {
        return result;
    }

    let mut after = window_start_exclusive;
    while let Some(candidate) = next_occurrence_after(rule, start_date, time_of_day, after) {
        if candidate > window_end_inclusive {
            break;
        }
        result.push(candidate);
        after = candidate;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn time(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        date(y, mo, d).and_time(time(h, mi))
    }

    #[test]
    fn one_time_future_date_returns_that_instant() {
        let rule = RecurrenceRule::OneTime;
        let start = date(2026, 3, 10);
        let t = time(9, 0);
        let after = at(2026, 3, 1, 0, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 3, 10, 9, 0)));
    }

    #[test]
    fn one_time_past_date_returns_none() {
        let rule = RecurrenceRule::OneTime;
        let start = date(2026, 3, 1);
        let t = time(9, 0);
        let after = at(2026, 3, 10, 0, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), None);
    }

    #[test]
    fn daily_same_day_if_time_not_passed() {
        let rule = RecurrenceRule::Daily;
        let start = date(2026, 1, 1);
        let t = time(18, 0);
        let after = at(2026, 3, 10, 9, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 3, 10, 18, 0)));
    }

    #[test]
    fn daily_rolls_to_tomorrow_if_time_passed() {
        let rule = RecurrenceRule::Daily;
        let start = date(2026, 1, 1);
        let t = time(9, 0);
        let after = at(2026, 3, 10, 18, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 3, 11, 9, 0)));
    }

    #[test]
    fn weekdays_friday_rolls_to_monday() {
        let rule = RecurrenceRule::Weekdays;
        let start = date(2026, 1, 1);
        let t = time(9, 0);
        // 2026-03-13 is a Friday.
        let after = at(2026, 3, 13, 18, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 3, 16, 9, 0)));
    }

    #[test]
    fn weekly_multiple_days_returns_nearest_selected_day() {
        let rule = RecurrenceRule::Weekly { days: vec![Weekday::Mon, Weekday::Wed, Weekday::Fri] };
        let start = date(2026, 1, 1);
        let t = time(9, 0);
        // 2026-03-16 is a Monday; expect Wednesday 2026-03-18 next.
        let after = at(2026, 3, 16, 18, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 3, 18, 9, 0)));
    }

    #[test]
    fn weekly_wraps_to_next_week_when_all_passed() {
        let rule = RecurrenceRule::Weekly { days: vec![Weekday::Mon, Weekday::Wed, Weekday::Fri] };
        let start = date(2026, 1, 1);
        let t = time(9, 0);
        // 2026-03-20 is a Friday (last selected day of that week); expect next Monday.
        let after = at(2026, 3, 20, 18, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 3, 23, 9, 0)));
    }

    #[test]
    fn fortnightly_same_day_if_time_not_passed() {
        let rule = RecurrenceRule::Fortnightly;
        let start = date(2026, 3, 3); // a Tuesday
        let t = time(18, 0);
        let after = at(2026, 3, 17, 9, 0); // two cycles later, same weekday
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 3, 17, 18, 0)));
    }

    #[test]
    fn fortnightly_rolls_to_next_cycle_if_time_passed() {
        let rule = RecurrenceRule::Fortnightly;
        let start = date(2026, 3, 3);
        let t = time(9, 0);
        let after = at(2026, 3, 17, 18, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 3, 31, 9, 0)));
    }

    #[test]
    fn fortnightly_skips_the_off_week() {
        let rule = RecurrenceRule::Fortnightly;
        let start = date(2026, 3, 3);
        let t = time(9, 0);
        // 2026-03-10 is the Tuesday in between cycles; must not match.
        let after = at(2026, 3, 4, 0, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 3, 17, 9, 0)));
    }

    #[test]
    fn fortnightly_before_start_date_returns_start_date() {
        let rule = RecurrenceRule::Fortnightly;
        let start = date(2026, 3, 3);
        let t = time(9, 0);
        let after = at(2026, 1, 1, 0, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 3, 3, 9, 0)));
    }

    #[test]
    fn monthly_returns_this_month_if_not_passed() {
        let rule = RecurrenceRule::MonthlyOnDay { day: 15 };
        let start = date(2026, 1, 1);
        let t = time(9, 0);
        let after = at(2026, 3, 1, 0, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 3, 15, 9, 0)));
    }

    #[test]
    fn monthly_rolls_to_next_month_if_passed() {
        let rule = RecurrenceRule::MonthlyOnDay { day: 15 };
        let start = date(2026, 1, 1);
        let t = time(9, 0);
        let after = at(2026, 3, 20, 0, 0);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 4, 15, 9, 0)));
    }

    #[test]
    fn monthly_day_31_clamps_to_feb_28_non_leap_year() {
        let rule = RecurrenceRule::MonthlyOnDay { day: 31 };
        let start = date(2025, 1, 1);
        let t = time(9, 0);
        let after = at(2025, 1, 31, 23, 59);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2025, 2, 28, 9, 0)));
    }

    #[test]
    fn monthly_day_31_clamps_to_feb_29_leap_year() {
        let rule = RecurrenceRule::MonthlyOnDay { day: 31 };
        let start = date(2028, 1, 1);
        let t = time(9, 0);
        let after = at(2028, 1, 31, 23, 59);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2028, 2, 29, 9, 0)));
    }

    #[test]
    fn monthly_day_31_in_30_day_month_clamps_to_30() {
        let rule = RecurrenceRule::MonthlyOnDay { day: 31 };
        let start = date(2026, 1, 1);
        let t = time(9, 0);
        let after = at(2026, 3, 31, 23, 59);
        assert_eq!(next_occurrence_after(&rule, start, t, after), Some(at(2026, 4, 30, 9, 0)));
    }

    #[test]
    fn due_instants_between_daily_returns_multiple_for_multi_day_gap() {
        let rule = RecurrenceRule::Daily;
        let start = date(2026, 1, 1);
        let t = time(9, 0);
        let window_start = at(2026, 3, 10, 0, 0);
        let window_end = at(2026, 3, 13, 0, 0);
        let instants = due_instants_between(&rule, start, t, window_start, window_end);
        assert_eq!(
            instants,
            vec![at(2026, 3, 10, 9, 0), at(2026, 3, 11, 9, 0), at(2026, 3, 12, 9, 0)]
        );
    }

    #[test]
    fn due_instants_between_one_time_returns_at_most_one() {
        let rule = RecurrenceRule::OneTime;
        let start = date(2026, 3, 11);
        let t = time(9, 0);
        let window_start = at(2026, 3, 1, 0, 0);
        let window_end = at(2026, 3, 31, 0, 0);
        assert_eq!(
            due_instants_between(&rule, start, t, window_start, window_end),
            vec![at(2026, 3, 11, 9, 0)]
        );
    }

    #[test]
    fn due_instants_between_empty_window_returns_empty() {
        let rule = RecurrenceRule::Daily;
        let start = date(2026, 1, 1);
        let t = time(9, 0);
        let instant = at(2026, 3, 10, 0, 0);
        assert_eq!(due_instants_between(&rule, start, t, instant, instant), Vec::new());
        assert_eq!(due_instants_between(&rule, start, t, at(2026, 3, 10, 5, 0), at(2026, 3, 9, 5, 0)), Vec::new());
    }
}
