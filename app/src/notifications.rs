//! Native GNOME notification wiring: firing an Urgent-priority, action-button-only
//! notification per due occurrence, and parsing the "Done" action's target back into the
//! reminder/occurrence it refers to.
//!
//! The notification id is unique *per occurrence* (not just per reminder) — that's what
//! makes GNOME Shell stack overlapping/simultaneous reminders as separate notifications
//! instead of one replacing another, since GNOME replaces a notification only when a new
//! one reuses the same id from the same app.

use chrono::NaiveDateTime;
use gio::prelude::*;
use glib::object::IsA;

const ID_TIMESTAMP_FMT: &str = "%Y%m%d%H%M%S";

pub fn notification_id(reminder_id: i64, due_instant: NaiveDateTime) -> String {
    format!("reminder-{reminder_id}-{}", due_instant.format(ID_TIMESTAMP_FMT))
}

/// Parses a notification id produced by `notification_id` back into its reminder id and
/// due instant. The timestamp suffix has no separators, so splitting on the *last* '-' is
/// unambiguous even though the reminder id portion could itself be large.
pub fn parse_notification_id(id: &str) -> Option<(i64, NaiveDateTime)> {
    let rest = id.strip_prefix("reminder-")?;
    let (reminder_id_str, timestamp_str) = rest.rsplit_once('-')?;
    let reminder_id = reminder_id_str.parse().ok()?;
    let due_instant = NaiveDateTime::parse_from_str(timestamp_str, ID_TIMESTAMP_FMT).ok()?;
    Some((reminder_id, due_instant))
}

pub fn send_reminder_notification(app: &impl IsA<gio::Application>, reminder_id: i64, title: &str, due_instant: NaiveDateTime) -> String {
    let id = notification_id(reminder_id, due_instant);
    let notification = gio::Notification::new(title);
    notification.set_body(Some(&format!("Scheduled for {}", due_instant.format("%-I:%M %p"))));
    notification.set_priority(gio::NotificationPriority::Urgent);
    notification.add_button_with_target_value("Snooze", "app.reminder-snooze", Some(&id.to_variant()));
    notification.add_button_with_target_value("Done", "app.reminder-done", Some(&id.to_variant()));
    app.send_notification(Some(&id), &notification);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    #[test]
    fn notification_id_roundtrips() {
        let id = notification_id(42, at(2026, 3, 10, 9, 0));
        assert_eq!(parse_notification_id(&id), Some((42, at(2026, 3, 10, 9, 0))));
    }

    #[test]
    fn distinct_occurrences_of_same_reminder_get_distinct_ids() {
        let first = notification_id(1, at(2026, 3, 10, 9, 0));
        let second = notification_id(1, at(2026, 3, 11, 9, 0));
        assert_ne!(first, second);
    }

    #[test]
    fn parse_notification_id_rejects_garbage() {
        assert_eq!(parse_notification_id("not-a-reminder-id"), None);
        assert_eq!(parse_notification_id(""), None);
    }
}
