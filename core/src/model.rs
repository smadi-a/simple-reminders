use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Weekday};

/// The five recurrence options the app supports. Intentionally a closed set —
/// see the project plan for why nothing else (yearly, custom intervals, etc.) is included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecurrenceRule {
    OneTime,
    Daily,
    Weekdays,
    Weekly { days: Vec<Weekday> },
    MonthlyOnDay { day: u8 },
}

impl RecurrenceRule {
    pub fn type_tag(&self) -> &'static str {
        match self {
            RecurrenceRule::OneTime => "one_time",
            RecurrenceRule::Daily => "daily",
            RecurrenceRule::Weekdays => "weekdays",
            RecurrenceRule::Weekly { .. } => "weekly",
            RecurrenceRule::MonthlyOnDay { .. } => "monthly",
        }
    }
}

/// All instants on `Reminder` (and in the `settings` table) are **local wall-clock**
/// datetimes, not UTC. This is a single-user local desktop app whose reminders fire at a
/// local wall-clock time by definition ("8:30am" means 8:30am wherever the user is right
/// now) — carrying a timezone through would add DST-edge-case complexity for no benefit
/// here, so everything downstream (recurrence math, the scheduler, the missed sweep)
/// consistently operates on `NaiveDateTime`/`NaiveDate`/`NaiveTime` in local time.
#[derive(Debug, Clone)]
pub struct Reminder {
    pub id: i64,
    pub title: String,
    pub time_of_day: NaiveTime,
    pub recurrence: RecurrenceRule,
    /// Default day for a new reminder is "today". For `OneTime` this is the literal due
    /// date; for recurring rules it's just the anchor the recurrence search starts from.
    pub start_date: NaiveDate,
    pub last_fired_due_instant: Option<NaiveDateTime>,
    pub missed: bool,
    pub missed_due_instant: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
}

/// A reminder plus the fields needed to insert it; `id`/`created_at` are assigned by the store.
#[derive(Debug, Clone)]
pub struct NewReminder {
    pub title: String,
    pub time_of_day: NaiveTime,
    pub recurrence: RecurrenceRule,
    pub start_date: NaiveDate,
}

/// Storage format shared by every persisted local-wall-clock instant (reminder
/// bookkeeping fields and the `settings` table alike), so there's one place that defines
/// how an instant round-trips through SQLite's TEXT storage.
pub const DATETIME_FMT: &str = "%Y-%m-%dT%H:%M:%S";

pub fn format_datetime(dt: NaiveDateTime) -> String {
    dt.format(DATETIME_FMT).to_string()
}

pub fn parse_datetime(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, DATETIME_FMT).ok()
}

pub fn weekday_to_code(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "MON",
        Weekday::Tue => "TUE",
        Weekday::Wed => "WED",
        Weekday::Thu => "THU",
        Weekday::Fri => "FRI",
        Weekday::Sat => "SAT",
        Weekday::Sun => "SUN",
    }
}

pub fn code_to_weekday(code: &str) -> Option<Weekday> {
    match code {
        "MON" => Some(Weekday::Mon),
        "TUE" => Some(Weekday::Tue),
        "WED" => Some(Weekday::Wed),
        "THU" => Some(Weekday::Thu),
        "FRI" => Some(Weekday::Fri),
        "SAT" => Some(Weekday::Sat),
        "SUN" => Some(Weekday::Sun),
        _ => None,
    }
}

pub fn encode_weekdays(days: &[Weekday]) -> String {
    days.iter()
        .map(|d| weekday_to_code(*d))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn decode_weekdays(encoded: &str) -> Vec<Weekday> {
    encoded
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(code_to_weekday)
        .collect()
}

/// The index math behind manual drag-to-reorder: returns `order` with `dragged_id` moved
/// to just before/after `target_id`. Kept pure and GTK-free here so it's unit-testable
/// without a UI harness — the UI only supplies the current on-screen order and the drop
/// location, then persists whatever this returns via `Store::reorder_reminders`.
pub fn move_id(order: &[i64], dragged_id: i64, target_id: i64, insert_after: bool) -> Vec<i64> {
    let mut new_order: Vec<i64> = order.iter().copied().filter(|&id| id != dragged_id).collect();
    let target_pos = new_order.iter().position(|&id| id == target_id).unwrap_or(new_order.len());
    let insert_at = if insert_after { target_pos + 1 } else { target_pos };
    new_order.insert(insert_at, dragged_id);
    new_order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekday_roundtrips_through_codes() {
        let days = vec![Weekday::Mon, Weekday::Wed, Weekday::Fri];
        let encoded = encode_weekdays(&days);
        assert_eq!(encoded, "MON,WED,FRI");
        assert_eq!(decode_weekdays(&encoded), days);
    }

    #[test]
    fn decode_weekdays_ignores_blank_and_unknown_tokens() {
        assert_eq!(decode_weekdays(""), Vec::<Weekday>::new());
        assert_eq!(decode_weekdays("MON,,BOGUS,FRI"), vec![Weekday::Mon, Weekday::Fri]);
    }

    #[test]
    fn move_id_inserts_before_target() {
        assert_eq!(move_id(&[1, 2, 3], 3, 1, false), vec![3, 1, 2]);
    }

    #[test]
    fn move_id_inserts_after_target() {
        assert_eq!(move_id(&[1, 2, 3], 1, 3, true), vec![2, 3, 1]);
    }

    #[test]
    fn move_id_within_adjacent_positions_is_a_no_op_shape() {
        assert_eq!(move_id(&[1, 2, 3], 2, 1, true), vec![1, 2, 3]);
    }

    #[test]
    fn move_id_appends_when_target_missing() {
        assert_eq!(move_id(&[1, 2, 3], 1, 99, false), vec![2, 3, 1]);
    }
}
