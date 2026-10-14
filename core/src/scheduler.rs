//! Ties the pure recurrence/missed-sweep logic to the `Store`: the live poll step the
//! background process ticks every ~20-30s, the one-shot login/missed sweep, and the
//! "Done" state transition shared by both the live-notification path and the in-GUI
//! "Mark as Done" action on a missed reminder.

use crate::db::Store;
use crate::error::Result;
use crate::missed::{compute_missed, MissedOccurrence};
use crate::model::{format_datetime, parse_datetime, Reminder, RecurrenceRule};
use crate::recurrence::{due_instants_between, next_occurrence_after};
use chrono::NaiveDateTime;

const LAST_SEEN_AT_KEY: &str = "last_seen_at";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiredReminder {
    pub reminder_id: i64,
    pub title: String,
    pub due_instant: NaiveDateTime,
}

/// One tick of the live poller: for every reminder, checks whether it has become newly
/// due since it last fired (or since it was created, if it has never fired), and if so
/// records that instant as fired and returns it so the caller can raise a notification.
/// Like the missed sweep, multiple due instants accumulated in one tick collapse to the
/// latest (this only matters if the process was asleep/suspended through more than one
/// occurrence; under normal ~20-30s polling there's at most one).
pub fn live_poll_step(store: &Store, now: NaiveDateTime) -> Result<Vec<FiredReminder>> {
    let mut fired = Vec::new();
    for reminder in store.list_reminders()? {
        let after = reminder.last_fired_due_instant.unwrap_or(reminder.created_at);
        let instants = due_instants_between(
            &reminder.recurrence,
            reminder.start_date,
            reminder.time_of_day,
            after,
            now,
        );
        if let Some(&due_instant) = instants.last() {
            store.set_last_fired(reminder.id, due_instant)?;
            fired.push(FiredReminder { reminder_id: reminder.id, title: reminder.title, due_instant });
        }
    }
    Ok(fired)
}

/// Runs once at background-mode startup: finds anything due between the last time the
/// app was known to be running (`settings.last_seen_at`) and now, flags each as missed,
/// and — critically — also stamps `last_fired_due_instant` for that occurrence so the
/// live poller doesn't immediately re-fire a duplicate notification for it afterward.
/// On a genuinely first-ever run (no `last_seen_at` yet) there is no prior session to
/// have missed anything during, so the window is empty and nothing is flagged.
pub fn run_missed_sweep(store: &Store, now: NaiveDateTime) -> Result<Vec<MissedOccurrence>> {
    let window_start = store
        .get_setting(LAST_SEEN_AT_KEY)?
        .and_then(|s| parse_datetime(&s))
        .unwrap_or(now);

    let reminders = store.list_reminders()?;
    let missed = compute_missed(&reminders, window_start, now);
    for occurrence in &missed {
        store.set_missed(occurrence.reminder_id, occurrence.due_instant)?;
        store.set_last_fired(occurrence.reminder_id, occurrence.due_instant)?;
    }
    store.set_setting(LAST_SEEN_AT_KEY, &format_datetime(now))?;
    Ok(missed)
}

/// Records `last_seen_at`; called on every live-poll tick (not just at clean shutdown) so
/// a crash doesn't widen the next startup's missed-sweep window beyond the real gap.
pub fn record_last_seen(store: &Store, now: NaiveDateTime) -> Result<()> {
    store.set_setting(LAST_SEEN_AT_KEY, &format_datetime(now))
}

/// Handles the "Done" action, whether it came from a live notification's button or from
/// the in-GUI "Mark as Done" on a missed reminder. A one-time reminder is simply deleted
/// (no history is kept); a recurring reminder just has its missed flag cleared — its next
/// occurrence is already in the future since it's always computed relative to "now".
pub fn mark_done(store: &Store, reminder_id: i64) -> Result<()> {
    let reminder = store.get_reminder(reminder_id)?;
    match reminder.recurrence {
        RecurrenceRule::OneTime => store.delete_reminder(reminder_id),
        RecurrenceRule::Daily
        | RecurrenceRule::Weekdays
        | RecurrenceRule::Weekly { .. }
        | RecurrenceRule::Fortnightly
        | RecurrenceRule::MonthlyOnDay { .. } => store.clear_missed(reminder_id),
    }
}

/// The "auto-arrange by date/time" order: every reminder's id, ascending by its next
/// occurrence after `now`. A reminder with no future occurrence (e.g. a stale one-time
/// reminder) sorts last; ties (including "no future occurrence" vs. "no future
/// occurrence") break on id, so the order is stable from one call to the next.
pub fn auto_sort_order(reminders: &[Reminder], now: NaiveDateTime) -> Vec<i64> {
    let mut keyed: Vec<(Option<NaiveDateTime>, i64)> = reminders
        .iter()
        .map(|r| (next_occurrence_after(&r.recurrence, r.start_date, r.time_of_day, now), r.id))
        .collect();
    keyed.sort_by(|(a_due, a_id), (b_due, b_id)| match (a_due, b_due) {
        (Some(a), Some(b)) => a.cmp(b).then(a_id.cmp(b_id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a_id.cmp(b_id),
    });
    keyed.into_iter().map(|(_, id)| id).collect()
}

/// Recomputes and persists the auto-arrange order. Callers are responsible for checking
/// `Store::auto_sort_enabled` first — this always (re)applies regardless of the setting.
pub fn apply_auto_sort(store: &Store, now: NaiveDateTime) -> Result<()> {
    let reminders = store.list_reminders()?;
    let order = auto_sort_order(&reminders, now);
    store.reorder_reminders(&order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NewReminder;
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

    #[test]
    fn mark_done_one_time_deletes_row() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_reminder(
                &NewReminder {
                    title: "Pay invoice".into(),
                    time_of_day: time(9, 0),
                    recurrence: RecurrenceRule::OneTime,
                    start_date: date(2026, 3, 10),
                },
                at(2026, 3, 1, 0, 0),
            )
            .unwrap();

        mark_done(&store, id).unwrap();
        assert!(store.get_reminder(id).is_err());
    }

    #[test]
    fn mark_done_recurring_keeps_row_and_clears_missed_flag() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_reminder(
                &NewReminder {
                    title: "Stand up".into(),
                    time_of_day: time(9, 0),
                    recurrence: RecurrenceRule::Daily,
                    start_date: date(2026, 3, 10),
                },
                at(2026, 3, 1, 0, 0),
            )
            .unwrap();
        store.set_missed(id, at(2026, 3, 11, 9, 0)).unwrap();

        mark_done(&store, id).unwrap();
        let fetched = store.get_reminder(id).unwrap();
        assert!(!fetched.missed);
        assert!(fetched.missed_due_instant.is_none());
    }

    #[test]
    fn mark_done_recurring_next_occurrence_computed_after_now() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_reminder(
                &NewReminder {
                    title: "Stand up".into(),
                    time_of_day: time(9, 0),
                    recurrence: RecurrenceRule::Daily,
                    start_date: date(2026, 3, 10),
                },
                at(2026, 3, 1, 0, 0),
            )
            .unwrap();
        store.set_last_fired(id, at(2026, 3, 11, 9, 0)).unwrap();
        mark_done(&store, id).unwrap();

        let fetched = store.get_reminder(id).unwrap();
        let next = due_instants_between(
            &fetched.recurrence,
            fetched.start_date,
            fetched.time_of_day,
            fetched.last_fired_due_instant.unwrap(),
            at(2026, 3, 13, 0, 0),
        );
        assert_eq!(next, vec![at(2026, 3, 12, 9, 0)]);
    }

    #[test]
    fn live_poll_fires_notification_exactly_once_for_single_due_instant() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_reminder(
                &NewReminder {
                    title: "Lunch".into(),
                    time_of_day: time(12, 0),
                    recurrence: RecurrenceRule::Daily,
                    start_date: date(2026, 3, 10),
                },
                at(2026, 3, 10, 0, 0),
            )
            .unwrap();

        let fired = live_poll_step(&store, at(2026, 3, 10, 12, 0)).unwrap();
        assert_eq!(fired, vec![FiredReminder { reminder_id: id, title: "Lunch".into(), due_instant: at(2026, 3, 10, 12, 0) }]);

        // Second immediate tick at the same instant must not re-fire.
        let fired_again = live_poll_step(&store, at(2026, 3, 10, 12, 0)).unwrap();
        assert_eq!(fired_again, Vec::new());
    }

    #[test]
    fn missed_sweep_sets_missed_flag_on_startup() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_reminder(
                &NewReminder {
                    title: "Water plants".into(),
                    time_of_day: time(9, 0),
                    recurrence: RecurrenceRule::Daily,
                    start_date: date(2026, 3, 1),
                },
                at(2026, 3, 1, 0, 0),
            )
            .unwrap();
        store.set_setting("last_seen_at", &format_datetime(at(2026, 3, 10, 0, 0))).unwrap();

        let missed = run_missed_sweep(&store, at(2026, 3, 12, 8, 0)).unwrap();
        assert_eq!(missed, vec![MissedOccurrence { reminder_id: id, due_instant: at(2026, 3, 11, 9, 0) }]);
        assert!(store.get_reminder(id).unwrap().missed);
    }

    #[test]
    fn missed_sweep_does_not_reflag_already_live_handled_occurrence() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_reminder(
                &NewReminder {
                    title: "Water plants".into(),
                    time_of_day: time(9, 0),
                    recurrence: RecurrenceRule::Daily,
                    start_date: date(2026, 3, 1),
                },
                at(2026, 3, 1, 0, 0),
            )
            .unwrap();
        // The live poller already fired and recorded today's occurrence before shutdown.
        store.set_last_fired(id, at(2026, 3, 10, 9, 0)).unwrap();
        store.set_setting("last_seen_at", &format_datetime(at(2026, 3, 10, 9, 0))).unwrap();

        // Sweep runs again right away (e.g. quick relaunch) with no new occurrence due yet.
        let missed = run_missed_sweep(&store, at(2026, 3, 10, 9, 5)).unwrap();
        assert_eq!(missed, Vec::new());
        assert!(!store.get_reminder(id).unwrap().missed);
    }

    #[test]
    fn auto_sort_order_ranks_by_next_occurrence_ascending() {
        let store = Store::open_in_memory().unwrap();
        let later = store
            .insert_reminder(
                &NewReminder {
                    title: "Later today".into(),
                    time_of_day: time(18, 0),
                    recurrence: RecurrenceRule::Daily,
                    start_date: date(2026, 3, 1),
                },
                at(2026, 3, 1, 0, 0),
            )
            .unwrap();
        let sooner = store
            .insert_reminder(
                &NewReminder {
                    title: "Sooner today".into(),
                    time_of_day: time(9, 0),
                    recurrence: RecurrenceRule::Daily,
                    start_date: date(2026, 3, 1),
                },
                at(2026, 3, 1, 0, 0),
            )
            .unwrap();
        let one_time_past = store
            .insert_reminder(
                &NewReminder {
                    title: "Stale one-time".into(),
                    time_of_day: time(9, 0),
                    recurrence: RecurrenceRule::OneTime,
                    start_date: date(2026, 2, 1),
                },
                at(2026, 2, 1, 0, 0),
            )
            .unwrap();

        let reminders = store.list_reminders().unwrap();
        // Midnight, so neither today's 9am nor 18:00 occurrence has passed yet.
        let order = auto_sort_order(&reminders, at(2026, 3, 10, 0, 0));
        assert_eq!(order, vec![sooner, later, one_time_past]);
    }

    #[test]
    fn apply_auto_sort_persists_the_computed_order() {
        let store = Store::open_in_memory().unwrap();
        let later = store
            .insert_reminder(
                &NewReminder {
                    title: "Later today".into(),
                    time_of_day: time(18, 0),
                    recurrence: RecurrenceRule::Daily,
                    start_date: date(2026, 3, 1),
                },
                at(2026, 3, 1, 0, 0),
            )
            .unwrap();
        let sooner = store
            .insert_reminder(
                &NewReminder {
                    title: "Sooner today".into(),
                    time_of_day: time(9, 0),
                    recurrence: RecurrenceRule::Daily,
                    start_date: date(2026, 3, 1),
                },
                at(2026, 3, 1, 0, 0),
            )
            .unwrap();

        apply_auto_sort(&store, at(2026, 3, 10, 0, 0)).unwrap();

        let ids: Vec<i64> = store.list_reminders().unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![sooner, later]);
    }

    #[test]
    fn missed_sweep_first_ever_run_has_no_prior_window_to_check() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_reminder(
                &NewReminder {
                    title: "Water plants".into(),
                    time_of_day: time(9, 0),
                    recurrence: RecurrenceRule::Daily,
                    start_date: date(2026, 3, 1),
                },
                at(2026, 3, 1, 0, 0),
            )
            .unwrap();

        assert!(store.get_setting("last_seen_at").unwrap().is_none());
        let missed = run_missed_sweep(&store, at(2026, 3, 12, 8, 0)).unwrap();
        assert_eq!(missed, Vec::new());
        assert!(store.get_setting("last_seen_at").unwrap().is_some());
    }
}
