//! SQLite-backed storage for reminders and app settings.
//!
//! By design (single-instance background process, see the project plan §6) exactly one
//! process — and, since it's driven off a single GLib main loop rather than worker
//! threads, exactly one thread — ever touches this connection. No connection pool or
//! locking is implemented because none is needed under that assumption.

use crate::error::{Error, Result};
use crate::model::{
    decode_weekdays, encode_weekdays, format_datetime, parse_datetime, NewReminder, Reminder,
    RecurrenceRule,
};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reminders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    time_of_day TEXT NOT NULL,
    recurrence_type TEXT NOT NULL
        CHECK(recurrence_type IN ('one_time','daily','weekdays','weekly','monthly')),
    start_date TEXT NOT NULL,
    weekly_days TEXT,
    monthly_day INTEGER,
    last_fired_due_instant TEXT,
    missed INTEGER NOT NULL DEFAULT 0,
    missed_due_instant TEXT,
    created_at TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

const DATE_FMT: &str = "%Y-%m-%d";
const TIME_FMT: &str = "%H:%M";

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    /// In-memory database — used by tests so they never touch the real filesystem.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Self::ensure_sort_order_column(&conn)?;
        Ok(Self { conn })
    }

    /// Databases created before manual reordering existed won't have `sort_order` —
    /// `CREATE TABLE IF NOT EXISTS` above doesn't retrofit columns onto an existing table,
    /// so add it here and backfill by `id` (i.e. original creation order).
    fn ensure_sort_order_column(conn: &Connection) -> Result<()> {
        let has_column = conn
            .prepare("SELECT 1 FROM pragma_table_info('reminders') WHERE name = 'sort_order'")?
            .exists([])?;
        if !has_column {
            conn.execute_batch(
                "ALTER TABLE reminders ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
                 UPDATE reminders SET sort_order = id;",
            )?;
        }
        Ok(())
    }

    /// `now` is passed in explicitly (rather than read from the real clock here) so the
    /// store stays fully deterministic/clock-free for tests, matching `recurrence.rs`.
    /// Callers pass `chrono::Local::now().naive_local()` in real use.
    pub fn insert_reminder(&self, new: &NewReminder, now: NaiveDateTime) -> Result<i64> {
        let (weekly_days, monthly_day) = encode_recurrence_extras(&new.recurrence);
        let next_order: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM reminders",
            [],
            |row| row.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO reminders
                (title, time_of_day, recurrence_type, start_date, weekly_days, monthly_day,
                 last_fired_due_instant, missed, missed_due_instant, created_at, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, NULL, ?7, ?8)",
            params![
                new.title,
                new.time_of_day.format(TIME_FMT).to_string(),
                new.recurrence.type_tag(),
                new.start_date.format(DATE_FMT).to_string(),
                weekly_days,
                monthly_day,
                format_datetime(now),
                next_order,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_reminder(&self, id: i64, new: &NewReminder) -> Result<()> {
        let (weekly_days, monthly_day) = encode_recurrence_extras(&new.recurrence);
        let changed = self.conn.execute(
            "UPDATE reminders
             SET title = ?1, time_of_day = ?2, recurrence_type = ?3, start_date = ?4,
                 weekly_days = ?5, monthly_day = ?6
             WHERE id = ?7",
            params![
                new.title,
                new.time_of_day.format(TIME_FMT).to_string(),
                new.recurrence.type_tag(),
                new.start_date.format(DATE_FMT).to_string(),
                weekly_days,
                monthly_day,
                id,
            ],
        )?;
        if changed == 0 {
            return Err(Error::NotFound(id));
        }
        Ok(())
    }

    pub fn delete_reminder(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM reminders WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_reminder(&self, id: i64) -> Result<Reminder> {
        self.conn
            .query_row(
                "SELECT id, title, time_of_day, recurrence_type, start_date, weekly_days,
                        monthly_day, last_fired_due_instant, missed, missed_due_instant, created_at
                 FROM reminders WHERE id = ?1",
                params![id],
                row_to_reminder,
            )
            .optional()?
            .ok_or(Error::NotFound(id))
    }

    pub fn list_reminders(&self) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, time_of_day, recurrence_type, start_date, weekly_days,
                    monthly_day, last_fired_due_instant, missed, missed_due_instant, created_at
             FROM reminders ORDER BY sort_order, id",
        )?;
        let rows = stmt.query_map([], row_to_reminder)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Error::from)
    }

    /// Persists a manual drag-to-reorder: `ordered_ids` must list every reminder's id in
    /// its new display order. Reminders not present are left with whatever `sort_order`
    /// they already had (callers always pass the full list in practice).
    pub fn reorder_reminders(&self, ordered_ids: &[i64]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (index, id) in ordered_ids.iter().enumerate() {
            tx.execute(
                "UPDATE reminders SET sort_order = ?1 WHERE id = ?2",
                params![index as i64, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn set_last_fired(&self, id: i64, instant: NaiveDateTime) -> Result<()> {
        self.conn.execute(
            "UPDATE reminders SET last_fired_due_instant = ?1 WHERE id = ?2",
            params![format_datetime(instant), id],
        )?;
        Ok(())
    }

    pub fn set_missed(&self, id: i64, instant: NaiveDateTime) -> Result<()> {
        self.conn.execute(
            "UPDATE reminders SET missed = 1, missed_due_instant = ?1 WHERE id = ?2",
            params![format_datetime(instant), id],
        )?;
        Ok(())
    }

    pub fn clear_missed(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE reminders SET missed = 0, missed_due_instant = NULL WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(Error::from)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

fn encode_recurrence_extras(rule: &RecurrenceRule) -> (Option<String>, Option<u8>) {
    match rule {
        RecurrenceRule::Weekly { days } => (Some(encode_weekdays(days)), None),
        RecurrenceRule::MonthlyOnDay { day } => (None, Some(*day)),
        RecurrenceRule::OneTime | RecurrenceRule::Daily | RecurrenceRule::Weekdays => (None, None),
    }
}

fn row_to_reminder(row: &Row) -> rusqlite::Result<Reminder> {
    let recurrence_type: String = row.get(3)?;
    let weekly_days: Option<String> = row.get(5)?;
    let monthly_day: Option<i64> = row.get(6)?;

    let recurrence = match recurrence_type.as_str() {
        "one_time" => RecurrenceRule::OneTime,
        "daily" => RecurrenceRule::Daily,
        "weekdays" => RecurrenceRule::Weekdays,
        "weekly" => RecurrenceRule::Weekly { days: decode_weekdays(weekly_days.as_deref().unwrap_or("")) },
        "monthly" => RecurrenceRule::MonthlyOnDay { day: monthly_day.unwrap_or(1) as u8 },
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                format!("unknown recurrence_type '{other}'").into(),
            ))
        }
    };

    let start_date: String = row.get(4)?;
    let time_of_day: String = row.get(2)?;
    let last_fired_due_instant: Option<String> = row.get(7)?;
    let missed_due_instant: Option<String> = row.get(9)?;
    let created_at: String = row.get(10)?;

    Ok(Reminder {
        id: row.get(0)?,
        title: row.get(1)?,
        time_of_day: NaiveTime::parse_from_str(&time_of_day, TIME_FMT)
            .map_err(|e| sql_parse_err(2, e))?,
        recurrence,
        start_date: NaiveDate::parse_from_str(&start_date, DATE_FMT).map_err(|e| sql_parse_err(4, e))?,
        last_fired_due_instant: last_fired_due_instant
            .map(|s| parse_datetime_col(&s, 7))
            .transpose()?,
        missed: row.get::<_, i64>(8)? != 0,
        missed_due_instant: missed_due_instant.map(|s| parse_datetime_col(&s, 9)).transpose()?,
        created_at: parse_datetime_col(&created_at, 10)?,
    })
}

fn parse_datetime_col(s: &str, col: usize) -> rusqlite::Result<NaiveDateTime> {
    parse_datetime(s).ok_or_else(|| sql_parse_err(col, format!("invalid stored datetime '{s}'")))
}

fn sql_parse_err(col: usize, msg: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NewReminder;
    use chrono::{NaiveDate, NaiveTime, Weekday};

    fn created_at() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 3, 1).unwrap().and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap())
    }

    fn sample(recurrence: RecurrenceRule) -> NewReminder {
        NewReminder {
            title: "Take out trash".into(),
            time_of_day: NaiveTime::from_hms_opt(8, 30, 0).unwrap(),
            recurrence,
            start_date: NaiveDate::from_ymd_opt(2026, 3, 10).unwrap(),
        }
    }

    #[test]
    fn insert_and_fetch_reminder_roundtrip_one_time() {
        let store = Store::open_in_memory().unwrap();
        let id = store.insert_reminder(&sample(RecurrenceRule::OneTime), created_at()).unwrap();
        let fetched = store.get_reminder(id).unwrap();
        assert_eq!(fetched.title, "Take out trash");
        assert_eq!(fetched.recurrence, RecurrenceRule::OneTime);
        assert_eq!(fetched.start_date, NaiveDate::from_ymd_opt(2026, 3, 10).unwrap());
        assert!(!fetched.missed);
    }

    #[test]
    fn insert_and_fetch_reminder_roundtrip_daily() {
        let store = Store::open_in_memory().unwrap();
        let id = store.insert_reminder(&sample(RecurrenceRule::Daily), created_at()).unwrap();
        assert_eq!(store.get_reminder(id).unwrap().recurrence, RecurrenceRule::Daily);
    }

    #[test]
    fn insert_and_fetch_reminder_roundtrip_weekdays() {
        let store = Store::open_in_memory().unwrap();
        let id = store.insert_reminder(&sample(RecurrenceRule::Weekdays), created_at()).unwrap();
        assert_eq!(store.get_reminder(id).unwrap().recurrence, RecurrenceRule::Weekdays);
    }

    #[test]
    fn insert_and_fetch_reminder_roundtrip_weekly() {
        let store = Store::open_in_memory().unwrap();
        let rule = RecurrenceRule::Weekly { days: vec![Weekday::Tue, Weekday::Thu] };
        let id = store.insert_reminder(&sample(rule.clone()), created_at()).unwrap();
        assert_eq!(store.get_reminder(id).unwrap().recurrence, rule);
    }

    #[test]
    fn insert_and_fetch_reminder_roundtrip_monthly() {
        let store = Store::open_in_memory().unwrap();
        let rule = RecurrenceRule::MonthlyOnDay { day: 31 };
        let id = store.insert_reminder(&sample(rule.clone()), created_at()).unwrap();
        assert_eq!(store.get_reminder(id).unwrap().recurrence, rule);
    }

    #[test]
    fn update_reminder_persists_changes() {
        let store = Store::open_in_memory().unwrap();
        let id = store.insert_reminder(&sample(RecurrenceRule::Daily), created_at()).unwrap();
        let mut updated = sample(RecurrenceRule::Weekdays);
        updated.title = "Water plants".into();
        store.update_reminder(id, &updated).unwrap();
        let fetched = store.get_reminder(id).unwrap();
        assert_eq!(fetched.title, "Water plants");
        assert_eq!(fetched.recurrence, RecurrenceRule::Weekdays);
    }

    #[test]
    fn update_reminder_missing_id_errors() {
        let store = Store::open_in_memory().unwrap();
        let err = store.update_reminder(999, &sample(RecurrenceRule::Daily));
        assert!(matches!(err, Err(Error::NotFound(999))));
    }

    #[test]
    fn delete_reminder_removes_row() {
        let store = Store::open_in_memory().unwrap();
        let id = store.insert_reminder(&sample(RecurrenceRule::OneTime), created_at()).unwrap();
        store.delete_reminder(id).unwrap();
        assert!(matches!(store.get_reminder(id), Err(Error::NotFound(_))));
    }

    #[test]
    fn list_reminders_returns_all() {
        let store = Store::open_in_memory().unwrap();
        store.insert_reminder(&sample(RecurrenceRule::OneTime), created_at()).unwrap();
        store.insert_reminder(&sample(RecurrenceRule::Daily), created_at()).unwrap();
        assert_eq!(store.list_reminders().unwrap().len(), 2);
    }

    #[test]
    fn list_reminders_orders_new_ones_last() {
        let store = Store::open_in_memory().unwrap();
        let first = store.insert_reminder(&sample(RecurrenceRule::OneTime), created_at()).unwrap();
        let second = store.insert_reminder(&sample(RecurrenceRule::Daily), created_at()).unwrap();
        let ids: Vec<i64> = store.list_reminders().unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![first, second]);
    }

    #[test]
    fn reorder_reminders_persists_new_order() {
        let store = Store::open_in_memory().unwrap();
        let first = store.insert_reminder(&sample(RecurrenceRule::OneTime), created_at()).unwrap();
        let second = store.insert_reminder(&sample(RecurrenceRule::Daily), created_at()).unwrap();
        let third = store.insert_reminder(&sample(RecurrenceRule::Weekdays), created_at()).unwrap();

        store.reorder_reminders(&[third, first, second]).unwrap();

        let ids: Vec<i64> = store.list_reminders().unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![third, first, second]);
    }

    #[test]
    fn settings_get_set_sound_path() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_setting("notification_sound_path").unwrap(), None);
        store.set_setting("notification_sound_path", "/usr/share/sounds/x.oga").unwrap();
        assert_eq!(
            store.get_setting("notification_sound_path").unwrap(),
            Some("/usr/share/sounds/x.oga".to_string())
        );
        // overwrite
        store.set_setting("notification_sound_path", "/usr/share/sounds/y.oga").unwrap();
        assert_eq!(
            store.get_setting("notification_sound_path").unwrap(),
            Some("/usr/share/sounds/y.oga".to_string())
        );
    }

    #[test]
    fn settings_last_seen_at_defaults_when_absent() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_setting("last_seen_at").unwrap(), None);
    }

    #[test]
    fn missed_flag_set_and_cleared() {
        let store = Store::open_in_memory().unwrap();
        let id = store.insert_reminder(&sample(RecurrenceRule::Daily), created_at()).unwrap();
        let instant = chrono::Local::now().naive_local();
        store.set_missed(id, instant).unwrap();
        let fetched = store.get_reminder(id).unwrap();
        assert!(fetched.missed);
        assert!(fetched.missed_due_instant.is_some());

        store.clear_missed(id).unwrap();
        let fetched = store.get_reminder(id).unwrap();
        assert!(!fetched.missed);
        assert!(fetched.missed_due_instant.is_none());
    }
}
