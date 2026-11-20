//! The SQLite file: opening, the schema (same as the Python app), backups
//! and the one-time move to the per-week tables.

use crate::logic::{self, needs_name, parse_date, DAYS};
use chrono::{Duration, Local};
use rusqlite::{params, types::ValueRef, Connection, OptionalExtension, Params};
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

pub const DEFAULT_SETTINGS: [(&str, &str); 15] = [
    ("school_start_date", ""),
    ("current_week_override", ""),
    ("current_school_day", ""),
    ("print_header_line1", "Weekly Assignment Schedule"),
    ("print_header_line2", ""),
    ("print_tagline", ""),
    ("print_footer_left", "Keep this sheet in your binder"),
    ("print_footer_center", ""),
    ("print_footer_right", ""),
    ("color_primary", "#1e3054"),
    ("color_accent", "#b8860b"),
    ("test_highlight_color", "#b5451b"),
    ("test_highlight_keywords", "test,quiz,exam,review"),
    ("day_adjust", "0"),
    ("week_adjust", "0"),
];

pub const PRINT_KEYS: [&str; 10] = [
    "print_header_line1",
    "print_header_line2",
    "print_tagline",
    "print_footer_left",
    "print_footer_center",
    "print_footer_right",
    "color_primary",
    "color_accent",
    "test_highlight_color",
    "test_highlight_keywords",
];

pub fn editable_setting(k: &str) -> bool {
    DEFAULT_SETTINGS.iter().any(|(d, _)| *d == k) && k != "current_week_override" && k != "current_school_day"
}

pub const KEEP_STARTUP_BACKUPS: usize = 15;
pub const KEEP_OTHER_BACKUPS: usize = 10;

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let c = Connection::open(path)?;
    c.busy_timeout(std::time::Duration::from_secs(10))?;
    c.execute_batch("PRAGMA trusted_schema=OFF;")?;
    Ok(c)
}

// ─────────────────────────────────────────────── rows as JSON objects ──
fn to_json(v: ValueRef) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
        ValueRef::Blob(_) => Value::Null,
    }
}

pub type Row = Map<String, Value>;

pub fn rows<P: Params>(c: &Connection, sql: &str, p: P) -> rusqlite::Result<Vec<Row>> {
    let mut st = c.prepare_cached(sql)?;
    let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
    let mut q = st.query(p)?;
    let mut out = Vec::new();
    while let Some(r) = q.next()? {
        let mut m = Map::new();
        for (i, n) in names.iter().enumerate() {
            m.insert(n.clone(), to_json(r.get_ref(i)?));
        }
        out.push(m);
    }
    Ok(out)
}

pub fn one<P: Params>(c: &Connection, sql: &str, p: P) -> rusqlite::Result<Option<Row>> {
    Ok(rows(c, sql, p)?.into_iter().next())
}

/// First column of every row as integers.
pub fn ids<P: Params>(c: &Connection, sql: &str, p: P) -> rusqlite::Result<Vec<i64>> {
    let mut st = c.prepare_cached(sql)?;
    let v = st.query_map(p, |r| r.get::<_, Option<i64>>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(v.into_iter().flatten().collect())
}

/// One text value (None for no row or NULL).
pub fn scalar_str<P: Params>(c: &Connection, sql: &str, p: P) -> rusqlite::Result<Option<String>> {
    c.query_row(sql, p, |r| r.get::<_, Option<String>>(0)).optional().map(|o| o.flatten())
}

pub fn count<P: Params>(c: &Connection, sql: &str, p: P) -> rusqlite::Result<i64> {
    c.query_row(sql, p, |r| r.get::<_, i64>(0))
}

pub fn default_setting(key: &str) -> &'static str {
    DEFAULT_SETTINGS.iter().find(|(k, _)| *k == key).map(|(_, v)| *v).unwrap_or("")
}

pub fn setting(c: &Connection, key: &str) -> rusqlite::Result<String> {
    Ok(scalar_str(c, "SELECT value FROM settings WHERE key=?", [key])?.unwrap_or_else(|| default_setting(key).to_string()))
}

pub fn all_settings(c: &Connection) -> rusqlite::Result<Map<String, Value>> {
    let mut out = Map::new();
    for (k, v) in DEFAULT_SETTINGS {
        out.insert(k.to_string(), json!(v));
    }
    let mut st = c.prepare_cached("SELECT key, value FROM settings")?;
    let mut q = st.query([])?;
    while let Some(r) = q.next()? {
        let k: Option<String> = r.get(0).ok();
        let v: Option<String> = r.get(1).ok().flatten();
        if let (Some(k), Some(v)) = (k, v) {
            out.insert(k, json!(v));
        }
    }
    Ok(out)
}

pub fn new_link() -> String {
    (0..12).map(|_| char::from_digit(fastrand::u32(0..16), 16).unwrap()).collect()
}

// ──────────────────────────────────────────────────────────── backups ──
pub fn backup_dir(db: &Path) -> PathBuf {
    db.parent().unwrap_or(Path::new(".")).join("backups")
}

/// Copy the database into backups/scheduler_<tag>_<time>.db (None if there is nothing to copy yet).
pub fn make_backup(db: &Path, tag: &str) -> Result<Option<PathBuf>, String> {
    match std::fs::metadata(db) {
        Ok(m) if m.len() > 0 => {}
        _ => return Ok(None),
    }
    let dir = backup_dir(db);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut dst = dir.join(format!("scheduler_{tag}_{}.db", Local::now().format("%Y-%m-%d_%H%M%S")));
    let mut n = 2;
    while dst.exists() {
        dst = dir.join(format!("scheduler_{tag}-{n}_{}.db", Local::now().format("%Y-%m-%d_%H%M%S")));
        n += 1;
    }
    let src = Connection::open(db).map_err(|e| e.to_string())?;
    src.busy_timeout(std::time::Duration::from_secs(10)).ok();
    src.backup(rusqlite::MAIN_DB, &dst, None).map_err(|e| e.to_string())?;
    prune_backups(&dir, tag);
    Ok(Some(dst))
}

fn prune_backups(dir: &Path, tag: &str) {
    let keep = if tag == "startup" { KEEP_STARTUP_BACKUPS } else { KEEP_OTHER_BACKUPS };
    let prefix = format!("scheduler_{tag}_");
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".db"))
        })
        .collect();
    old.sort();
    if old.len() > keep {
        for f in &old[..old.len() - keep] {
            let _ = std::fs::remove_file(f);
        }
    }
}

// ──────────────────────────────────────────────────────────── schema ──
const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT);
    CREATE TABLE IF NOT EXISTS grades (
        id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL,
        color TEXT DEFAULT '#1e3054', sort_order INTEGER DEFAULT 0);
    CREATE TABLE IF NOT EXISTS students (
        id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL,
        grade_id INTEGER NOT NULL, active INTEGER DEFAULT 1, sort_order INTEGER DEFAULT 0,
        FOREIGN KEY (grade_id) REFERENCES grades(id));
    CREATE TABLE IF NOT EXISTS schedule_entries (
        id INTEGER PRIMARY KEY AUTOINCREMENT, grade_id INTEGER NOT NULL,
        week_start TEXT NOT NULL DEFAULT '',
        day TEXT NOT NULL, subject TEXT NOT NULL, lesson_text TEXT NOT NULL,
        sort_order INTEGER DEFAULT 0);
    CREATE TABLE IF NOT EXISTS year_template (
        id INTEGER PRIMARY KEY AUTOINCREMENT, grade_id INTEGER NOT NULL,
        day TEXT NOT NULL, subject TEXT NOT NULL, lesson_text TEXT NOT NULL,
        sort_order INTEGER DEFAULT 0);
    CREATE TABLE IF NOT EXISTS weekly_assignments (
        id INTEGER PRIMARY KEY AUTOINCREMENT, subject TEXT NOT NULL,
        description TEXT NOT NULL, sort_order INTEGER DEFAULT 0);
    CREATE TABLE IF NOT EXISTS weekly_assignment_grades (
        weekly_id INTEGER NOT NULL, grade_id INTEGER NOT NULL,
        PRIMARY KEY (weekly_id, grade_id));
    CREATE TABLE IF NOT EXISTS teacher_notes (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        day TEXT NOT NULL UNIQUE, note_text TEXT NOT NULL DEFAULT '');
    CREATE TABLE IF NOT EXISTS entry_exclusions (
        entry_id INTEGER NOT NULL, student_id INTEGER NOT NULL,
        PRIMARY KEY (entry_id, student_id));
";

const SCHEMA_V2: &str = "
    CREATE TABLE IF NOT EXISTS weekly_items (
        id INTEGER PRIMARY KEY AUTOINCREMENT, week_start TEXT NOT NULL,
        subject TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
        sort_order INTEGER DEFAULT 0);
    CREATE TABLE IF NOT EXISTS weekly_item_grades (
        item_id INTEGER NOT NULL, grade_id INTEGER NOT NULL,
        PRIMARY KEY (item_id, grade_id));
    CREATE TABLE IF NOT EXISTS week_notes (
        week_start TEXT NOT NULL, day TEXT NOT NULL, note_text TEXT NOT NULL DEFAULT '',
        PRIMARY KEY (week_start, day));
    CREATE TABLE IF NOT EXISTS no_school_days (
        day_date TEXT PRIMARY KEY, label TEXT NOT NULL DEFAULT '');
    CREATE INDEX IF NOT EXISTS idx_entries_week ON schedule_entries (week_start, grade_id, day);
    CREATE INDEX IF NOT EXISTS idx_weekly_items_week ON weekly_items (week_start);
";

/// Create or update the tables. Safe to run on every start and on any old scheduler.db.
pub fn init_db(path: &Path) -> Result<(), String> {
    let fresh = std::fs::metadata(path).map(|m| m.len() == 0).unwrap_or(true);
    let c = open(path).map_err(|e| e.to_string())?;
    let r: rusqlite::Result<()> = (|| {
        c.execute_batch(SCHEMA)?;
        let mut st = c.prepare("PRAGMA table_info(schedule_entries)")?;
        let cols: Vec<String> = st.query_map([], |r| r.get::<_, String>(1))?.collect::<Result<_, _>>()?;
        drop(st);
        for (col, ddl) in [("week_start", "TEXT DEFAULT ''"), ("student_id", "INTEGER"), ("link_id", "TEXT")] {
            if !cols.iter().any(|c| c == col) {
                c.execute_batch(&format!("ALTER TABLE schedule_entries ADD COLUMN {col} {ddl}"))?;
            }
        }
        c.execute_batch(SCHEMA_V2)?;
        for (k, v) in DEFAULT_SETTINGS {
            c.execute("INSERT OR IGNORE INTO settings (key,value) VALUES (?,?)", [k, v])?;
        }
        c.execute(
            "UPDATE schedule_entries SET week_start=? WHERE week_start='' OR week_start IS NULL",
            [logic::monday_of_today()],
        )?;
        Ok(())
    })();
    r.map_err(|e| e.to_string())?;

    let ver = logic::loose_int(&setting(&c, "schema_version").map_err(|e| e.to_string())?, 0);
    if ver < 2 {
        let c = if !fresh {
            drop(c);
            make_backup(path, "before-v2")?;
            open(path).map_err(|e| e.to_string())?
        } else {
            c
        };
        c.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
        match migrate_to_v2(&c) {
            Ok(()) => c.execute_batch("COMMIT").map_err(|e| e.to_string())?,
            Err(e) => {
                let _ = c.execute_batch("ROLLBACK");
                return Err(e.to_string());
            }
        }
    }
    Ok(())
}

/// One-time move of the old single-set data into the per-week tables.
fn migrate_to_v2(c: &Connection) -> rusqlite::Result<()> {
    let today_mon = logic::monday_of_today();
    let latest = scalar_str(c, "SELECT MAX(week_start) m FROM schedule_entries", [])?;
    let mut target: BTreeSet<String> = BTreeSet::new();
    target.insert(today_mon);
    if let Some(l) = latest.filter(|l| !l.is_empty()) {
        target.insert(l);
    }

    // Weekly assignments & teacher notes were one shared set; give that set to
    // this week and the latest planned week, so both look exactly like before.
    let old_weekly = rows(c, "SELECT * FROM weekly_assignments ORDER BY sort_order,id", [])?;
    for ws in &target {
        if count(c, "SELECT COUNT(*) FROM weekly_items WHERE week_start=?", [ws])? > 0 {
            continue;
        }
        for (i, w) in old_weekly.iter().enumerate() {
            c.execute(
                "INSERT INTO weekly_items (week_start,subject,description,sort_order) VALUES (?,?,?,?)",
                params![ws, w["subject"].as_str().unwrap_or(""), w["description"].as_str().unwrap_or(""), i as i64],
            )?;
            let nid = c.last_insert_rowid();
            for g in ids(c, "SELECT grade_id FROM weekly_assignment_grades WHERE weekly_id=?", [w["id"].as_i64()])? {
                c.execute("INSERT OR IGNORE INTO weekly_item_grades VALUES (?,?)", [nid, g])?;
            }
        }
    }
    let notes: HashMap<String, String> = {
        let mut st = c.prepare("SELECT day,note_text FROM teacher_notes")?;
        let v = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default())))?
            .collect::<Result<Vec<_>, _>>()?;
        v.into_iter().collect()
    };
    for ws in &target {
        for d in DAYS {
            if let Some(t) = notes.get(d).filter(|t| !t.trim().is_empty()) {
                c.execute("INSERT OR IGNORE INTO week_notes (week_start,day,note_text) VALUES (?,?,?)", [ws, d, t])?;
            }
        }
    }

    // Guess days off: a weekday with nothing scheduled in a week that is
    // otherwise scheduled, between the start date and today.
    if let Some(start) = parse_date(&setting(c, "school_start_date")?) {
        let today = logic::today();
        let mut per_week: Vec<(String, Vec<String>)> = Vec::new();
        {
            let mut st = c.prepare("SELECT DISTINCT week_start, day FROM schedule_entries")?;
            let v = st
                .query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            for (ws, d) in v {
                let (ws, d) = (ws.unwrap_or_default(), d.unwrap_or_default());
                match per_week.iter_mut().find(|(w, _)| *w == ws) {
                    Some((_, days)) => days.push(d),
                    None => per_week.push((ws, vec![d])),
                }
            }
        }
        for (ws, used) in per_week {
            let Some(mon) = parse_date(&ws) else { continue };
            if used.is_empty() {
                continue;
            }
            for (i, d) in DAYS.iter().enumerate() {
                let dt = mon + Duration::days(i as i64);
                if !used.iter().any(|u| u == d) && start <= dt && dt <= today {
                    c.execute(
                        "INSERT OR IGNORE INTO no_school_days (day_date,label) VALUES (?,?)",
                        [logic::iso(dt).as_str(), "No school (found empty in your schedule)"],
                    )?;
                }
            }
        }
    }

    // Link identical entries across grades in every week.
    let weeks: Vec<String> = {
        let mut st = c.prepare("SELECT DISTINCT week_start FROM schedule_entries")?;
        let v = st.query_map([], |r| r.get::<_, Option<String>>(0))?.collect::<Result<Vec<_>, _>>()?;
        v.into_iter().flatten().collect()
    };
    for ws in weeks {
        autolink_week(c, &ws)?;
    }
    c.execute("INSERT OR REPLACE INTO settings (key,value) VALUES (?,?)", ["schema_version", "2"])?;
    Ok(())
}

/// Link class-wide entries that are identical (same day, subject and lesson)
/// across two or more grades. Blank lessons are never linked, since two empty
/// "English" cells are usually different courses.
pub fn autolink_week(c: &Connection, ws: &str) -> rusqlite::Result<i64> {
    let all = rows(
        c,
        "SELECT id,grade_id,day,subject,lesson_text,link_id FROM schedule_entries \
         WHERE week_start=? AND student_id IS NULL AND TRIM(lesson_text)<>'' ORDER BY sort_order,id",
        [ws],
    )?;
    let mut keys: Vec<(String, String, String)> = Vec::new();
    let mut groups: HashMap<(String, String, String), Vec<Row>> = HashMap::new();
    for r in all {
        let lesson = r["lesson_text"].as_str().unwrap_or("").to_string();
        if needs_name(&lesson) {
            continue; // "Lesson 42 ~ " in two grades is not the same lesson
        }
        let key = (
            r["day"].as_str().unwrap_or("").to_string(),
            r["subject"].as_str().unwrap_or("").trim().to_lowercase(),
            lesson.trim().to_string(),
        );
        if !groups.contains_key(&key) {
            keys.push(key.clone());
        }
        groups.entry(key).or_default().push(r);
    }
    let mut made = 0;
    for k in keys {
        let members = &groups[&k];
        let mut ms: Vec<&Row> = Vec::new();
        for m in members {
            if !ms.iter().any(|x| x["grade_id"] == m["grade_id"]) {
                ms.push(m);
            }
        }
        if ms.len() < 2 {
            continue;
        }
        let existing: BTreeSet<String> =
            ms.iter().filter_map(|m| m["link_id"].as_str()).filter(|s| !s.is_empty()).map(String::from).collect();
        if existing.len() > 1 {
            continue;
        }
        let lid = existing.into_iter().next().unwrap_or_else(new_link);
        for m in ms {
            if m["link_id"].as_str().is_none_or(|s| s.is_empty()) {
                c.execute("UPDATE schedule_entries SET link_id=? WHERE id=?", params![lid, m["id"].as_i64()])?;
                made += 1;
            }
        }
    }
    Ok(made)
}
