//! Human-readable descriptions of a reminder's schedule, used as the list row subtitle.

use crate::model::RecurrenceRule;
use chrono::{NaiveDate, NaiveTime, Weekday};

const WEEK_ORDER: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

fn format_time(time_of_day: NaiveTime) -> String {
    time_of_day.format("%-I:%M %p").to_string()
}

pub fn describe_recurrence(
    recurrence: &RecurrenceRule,
    start_date: NaiveDate,
    time_of_day: NaiveTime,
    today: NaiveDate,
) -> String {
    let t = format_time(time_of_day);
    match recurrence {
        RecurrenceRule::OneTime => {
            let day = if start_date == today {
                "Today".to_string()
            } else if start_date == today.succ_opt().unwrap() {
                "Tomorrow".to_string()
            } else {
                start_date.format("%b %-d, %Y").to_string()
            };
            format!("{day} at {t}")
        }
        RecurrenceRule::Daily => format!("Daily at {t}"),
        RecurrenceRule::Weekdays => format!("Weekdays at {t}"),
        RecurrenceRule::Weekly { days } => {
            let ordered: Vec<String> = WEEK_ORDER
                .iter()
                .filter(|d| days.contains(d))
                .map(|d| d.to_string())
                .collect();
            format!("Weekly: {} at {t}", ordered.join(", "))
        }
        RecurrenceRule::Fortnightly => format!("Every 2 weeks at {t}"),
        RecurrenceRule::MonthlyOnDay { day } => format!("Monthly on day {day} at {t}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Weekday;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn time(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn describes_one_time() {
        assert_eq!(
            describe_recurrence(&RecurrenceRule::OneTime, date(2026, 3, 10), time(9, 0), date(2026, 1, 1)),
            "Mar 10, 2026 at 9:00 AM"
        );
    }

    #[test]
    fn describes_one_time_today() {
        assert_eq!(
            describe_recurrence(&RecurrenceRule::OneTime, date(2026, 3, 10), time(9, 0), date(2026, 3, 10)),
            "Today at 9:00 AM"
        );
    }

    #[test]
    fn describes_one_time_tomorrow() {
        assert_eq!(
            describe_recurrence(&RecurrenceRule::OneTime, date(2026, 3, 10), time(9, 0), date(2026, 3, 9)),
            "Tomorrow at 9:00 AM"
        );
    }

    #[test]
    fn describes_daily() {
        assert_eq!(
            describe_recurrence(&RecurrenceRule::Daily, date(2026, 3, 10), time(18, 0), date(2026, 1, 1)),
            "Daily at 6:00 PM"
        );
    }

    #[test]
    fn describes_weekdays() {
        assert_eq!(
            describe_recurrence(&RecurrenceRule::Weekdays, date(2026, 3, 10), time(7, 0), date(2026, 1, 1)),
            "Weekdays at 7:00 AM"
        );
    }

    #[test]
    fn describes_weekly_in_week_order_regardless_of_input_order() {
        let rule = RecurrenceRule::Weekly { days: vec![Weekday::Fri, Weekday::Mon, Weekday::Wed] };
        assert_eq!(
            describe_recurrence(&rule, date(2026, 3, 10), time(18, 0), date(2026, 1, 1)),
            "Weekly: Mon, Wed, Fri at 6:00 PM"
        );
    }

    #[test]
    fn describes_fortnightly() {
        assert_eq!(
            describe_recurrence(&RecurrenceRule::Fortnightly, date(2026, 3, 10), time(18, 0), date(2026, 1, 1)),
            "Every 2 weeks at 6:00 PM"
        );
    }

    #[test]
    fn describes_monthly() {
        let rule = RecurrenceRule::MonthlyOnDay { day: 31 };
        assert_eq!(
            describe_recurrence(&rule, date(2026, 3, 10), time(8, 30), date(2026, 1, 1)),
            "Monthly on day 31 at 8:30 AM"
        );
    }
}
