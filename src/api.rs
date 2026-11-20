//! The editor's JSON API. Each handler is a port of the matching Flask route
//! in the Python app, so both versions make the same changes to scheduler.db.

use crate::db::{self, all_settings, autolink_week, count, ids, new_link, one, rows, scalar_str, Row};
use crate::logic::{self, add, day_idx, fmt_week_range, lesson_text_for, parse_date, parse_lesson, Calendar, DAYS};
use chrono::Duration;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

pub struct ApiErr(pub u16, pub String);

impl From<rusqlite::Error> for ApiErr {
    fn from(e: rusqlite::Error) -> Self {
        ApiErr(500, format!("database error: {e}"))
    }
}

pub type Res = Result<Value, ApiErr>;
pub type Body = Map<String, Value>;

fn bad(msg: &str) -> ApiErr {
    ApiErr(400, msg.to_string())
}
fn not_found() -> ApiErr {
    ApiErr(404, "not found".into())
}

// ─────────────────────────────────────────────── loose JSON helpers ──
/// Python truthiness.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
static NULL: Value = Value::Null;
fn get<'a>(b: &'a Body, k: &str) -> &'a Value {
    b.get(k).unwrap_or(&NULL)
}
fn has(b: &Body, k: &str) -> bool {
    b.contains_key(k)
}
/// `int(v)`
pub fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(*b as i64),
        _ => None,
    }
}
fn req_int(b: &Body, k: &str) -> Result<i64, ApiErr> {
    as_int(get(b, k)).ok_or_else(|| bad(&format!("{k} is missing")))
}
/// `b.get(k) or None`, as a number
fn opt_id(b: &Body, k: &str) -> Option<i64> {
    let v = get(b, k);
    if truthy(v) {
        as_int(v)
    } else {
        None
    }
}
fn as_text(v: &Value) -> Option<&str> {
    v.as_str()
}
/// `b.get(k) or ''`
fn text_or_empty(b: &Body, k: &str) -> String {
    match get(b, k) {
        Value::String(s) => s.clone(),
        v if truthy(v) => v.to_string(),
        _ => String::new(),
    }
}
fn int_list(v: &Value) -> Vec<i64> {
    v.as_array().map(|a| a.iter().filter_map(as_int).collect()).unwrap_or_default()
}
fn week_of(b: &Body) -> String {
    logic::parse_week(as_text(get(b, "week")))
}

fn fi(r: &Row, k: &str) -> i64 {
    r.get(k).and_then(|v| v.as_i64()).unwrap_or(0)
}
fn fo(r: &Row, k: &str) -> Option<i64> {
    r.get(k).and_then(|v| v.as_i64()).filter(|v| *v != 0)
}
fn fs<'a>(r: &'a Row, k: &str) -> &'a str {
    r.get(k).and_then(|v| v.as_str()).unwrap_or("")
}
fn fl(r: &Row) -> Option<String> {
    r.get("link_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from)
}
fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

// ─────────────────────────────────────────────────────── entry logic ──
pub fn week_entries(c: &Connection, ws: &str) -> rusqlite::Result<Vec<Row>> {
    let mut ents = rows(c, "SELECT * FROM schedule_entries WHERE week_start=? ORDER BY sort_order,id", [ws])?;
    let mut excl: HashMap<i64, Vec<i64>> = HashMap::new();
    {
        let mut st = c.prepare_cached(
            "SELECT x.entry_id, x.student_id FROM entry_exclusions x \
             JOIN schedule_entries e ON e.id = x.entry_id WHERE e.week_start=?",
        )?;
        let mut q = st.query([ws])?;
        while let Some(r) = q.next()? {
            if let (Ok(e), Ok(s)) = (r.get::<_, i64>(0), r.get::<_, i64>(1)) {
                excl.entry(e).or_default().push(s);
            }
        }
    }
    for e in ents.iter_mut() {
        let mut x = excl.remove(&fi(e, "id")).unwrap_or_default();
        x.sort();
        e.insert("excluded".into(), json!(x));
    }
    Ok(ents)
}

fn exclusions(c: &Connection, eid: i64) -> rusqlite::Result<Vec<i64>> {
    ids(c, "SELECT student_id FROM entry_exclusions WHERE entry_id=?", [eid])
}

fn find_entry(c: &Connection, eid: i64) -> rusqlite::Result<Option<Row>> {
    let Some(mut e) = one(c, "SELECT * FROM schedule_entries WHERE id=?", [eid])? else { return Ok(None) };
    e.insert("excluded".into(), json!(exclusions(c, eid)?));
    Ok(Some(e))
}

fn get_entry(c: &Connection, eid: i64) -> Result<Row, ApiErr> {
    find_entry(c, eid)?.ok_or_else(not_found)
}

fn entries_by_ids(c: &Connection, list: &[i64]) -> rusqlite::Result<Value> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &i in list {
        if seen.insert(i) {
            if let Some(e) = find_entry(c, i)? {
                out.push(Value::Object(e));
            }
        }
    }
    Ok(Value::Array(out))
}

fn changed(c: &Connection, list: &[i64]) -> Res {
    Ok(json!({ "changed": entries_by_ids(c, list)? }))
}

fn free_slot(c: &Connection, gid: i64, ws: &str, day: &str, preferred: Option<i64>) -> rusqlite::Result<i64> {
    let used: HashSet<i64> =
        ids(c, "SELECT sort_order FROM schedule_entries WHERE grade_id=? AND week_start=? AND day=?", params![gid, ws, day])?
            .into_iter()
            .collect();
    if let Some(p) = preferred {
        if !used.contains(&p) {
            return Ok(p);
        }
    }
    Ok(used.iter().max().map(|m| m + 1).unwrap_or(0))
}

fn slot_taken(c: &Connection, gid: i64, ws: &str, day: &str, slot: i64, exclude: Option<i64>) -> rusqlite::Result<Option<i64>> {
    let sql = "SELECT id FROM schedule_entries WHERE grade_id=? AND week_start=? AND day=? AND sort_order=? AND id<>?";
    c.query_row(sql, params![gid, ws, day, slot, exclude.unwrap_or(-1)], |r| r.get(0)).optional()
}

#[allow(clippy::too_many_arguments)]
fn insert_entry(
    c: &Connection, gid: i64, ws: &str, day: &str, slot: i64, subject: &str, lesson: &str, sid: Option<i64>,
    link: Option<&str>,
) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO schedule_entries (grade_id,week_start,day,subject,lesson_text,sort_order,student_id,link_id) \
         VALUES (?,?,?,?,?,?,?,?)",
        params![gid, ws, day, subject, lesson, slot, sid, link],
    )?;
    Ok(c.last_insert_rowid())
}

fn set_exclusions(c: &Connection, eid: i64, students: &[i64]) -> rusqlite::Result<()> {
    c.execute("DELETE FROM entry_exclusions WHERE entry_id=?", [eid])?;
    for s in students {
        c.execute("INSERT OR IGNORE INTO entry_exclusions (entry_id,student_id) VALUES (?,?)", [eid, *s])?;
    }
    Ok(())
}

fn link_members(c: &Connection, link: Option<&str>, exclude: Option<i64>) -> rusqlite::Result<Vec<Row>> {
    let Some(l) = link.filter(|l| !l.is_empty()) else { return Ok(vec![]) };
    rows(c, "SELECT * FROM schedule_entries WHERE link_id=? AND id<>?", params![l, exclude.unwrap_or(-1)])
}

fn member_ids(c: &Connection, link: Option<&str>, exclude: Option<i64>) -> rusqlite::Result<Vec<i64>> {
    Ok(link_members(c, link, exclude)?.iter().map(|m| fi(m, "id")).collect())
}

/// A link with only one member left is no link at all.
fn tidy_link(c: &Connection, link: Option<&str>) -> rusqlite::Result<()> {
    if let Some(l) = link.filter(|l| !l.is_empty()) {
        if link_members(c, Some(l), None)?.len() < 2 {
            c.execute("UPDATE schedule_entries SET link_id=NULL WHERE link_id=?", [l])?;
        }
    }
    Ok(())
}

pub fn weekly_for(c: &Connection, ws: &str) -> rusqlite::Result<Vec<Row>> {
    let mut items = rows(c, "SELECT * FROM weekly_items WHERE week_start=? ORDER BY sort_order,id", [ws])?;
    let mut gmap: HashMap<i64, Vec<i64>> = HashMap::new();
    {
        let mut st = c.prepare_cached(
            "SELECT g.item_id, g.grade_id FROM weekly_item_grades g \
             JOIN weekly_items w ON w.id = g.item_id WHERE w.week_start=?",
        )?;
        let mut q = st.query([ws])?;
        while let Some(r) = q.next()? {
            if let (Ok(i), Ok(g)) = (r.get::<_, i64>(0), r.get::<_, i64>(1)) {
                gmap.entry(i).or_default().push(g);
            }
        }
    }
    for i in items.iter_mut() {
        let g = gmap.remove(&fi(i, "id")).unwrap_or_default();
        i.insert("grade_ids".into(), json!(g));
    }
    Ok(items)
}

pub fn notes_for(c: &Connection, ws: &str) -> rusqlite::Result<Map<String, Value>> {
    let mut out = Map::new();
    let mut st = c.prepare_cached("SELECT day,note_text FROM week_notes WHERE week_start=?")?;
    let mut q = st.query([ws])?;
    while let Some(r) = q.next()? {
        let d: String = r.get(0)?;
        let t: Option<String> = r.get(1)?;
        out.insert(d, json!(t.unwrap_or_default()));
    }
    Ok(out)
}

fn history_weeks(c: &Connection) -> rusqlite::Result<Value> {
    let cur = logic::monday_of_today();
    let r = rows(
        c,
        "SELECT week_start, COUNT(*) cnt FROM schedule_entries WHERE week_start<>'' \
         GROUP BY week_start ORDER BY week_start DESC",
        [],
    )?;
    Ok(Value::Array(
        r.iter()
            .map(|r| {
                let ws = fs(r, "week_start");
                json!({"week_start": ws, "label": fmt_week_range(ws), "count": fi(r, "cnt"), "is_current": ws == cur})
            })
            .collect(),
    ))
}

// ───────────────────────────────────────────────────────── page data ──
pub fn state(c: &Connection, week: Option<&str>, data_dir: &str, share_url: Option<&str>) -> Res {
    let ws = logic::parse_week(week);
    let settings = all_settings(c)?;
    let grades = rows(c, "SELECT * FROM grades ORDER BY sort_order,id", [])?;
    let students = rows(c, "SELECT * FROM students WHERE active=1 ORDER BY sort_order,name", [])?;
    let cal = Calendar::load(c)?;
    let info = logic::week_info(&ws, &cal);
    let cur = logic::monday_of_today();
    let mon = parse_date(&ws).unwrap();
    let prev_ws = logic::iso(mon - Duration::days(7));
    let next_ws = logic::iso(mon + Duration::days(7));
    let src = scalar_str(c, "SELECT MAX(week_start) m FROM schedule_entries WHERE week_start<?", [&ws])?
        .filter(|s| !s.is_empty());
    let mut all_ids = ids(c, "SELECT id FROM students", [])?;
    all_ids.sort();
    let orphans = count(
        c,
        "SELECT COUNT(*) n FROM schedule_entries WHERE student_id IS NOT NULL \
         AND student_id NOT IN (SELECT id FROM students)",
        [],
    )? + count(
        c,
        "SELECT COUNT(*) n FROM entry_exclusions WHERE student_id NOT IN (SELECT id FROM students) \
         OR entry_id NOT IN (SELECT id FROM schedule_entries)",
        [],
    )?;
    let prev_has_weekly = count(c, "SELECT COUNT(*) FROM (SELECT 1 FROM weekly_items WHERE week_start<? LIMIT 1)", [&ws])? > 0;

    let mut m = Map::new();
    m.insert("week".into(), json!(ws));
    m.insert("week_label".into(), json!(fmt_week_range(&ws)));
    m.insert("prev_week".into(), json!(prev_ws));
    m.insert("next_week".into(), json!(next_ws));
    m.insert("current_week".into(), json!(cur));
    m.insert("is_current_week".into(), json!(ws == cur));
    let label = src.as_deref().map(fmt_week_range);
    m.insert("source_week".into(), json!(src));
    m.insert("source_label".into(), json!(label));
    m.insert("prev_has_weekly".into(), json!(prev_has_weekly));
    m.insert("days".into(), json!(DAYS));
    for (k, v) in info {
        m.insert(k, v);
    }
    m.insert("grades".into(), json!(grades));
    m.insert("students".into(), json!(students));
    m.insert("all_student_ids".into(), json!(all_ids));
    m.insert("entries".into(), json!(week_entries(c, &ws)?));
    m.insert("weekly".into(), json!(weekly_for(c, &ws)?));
    m.insert("notes".into(), Value::Object(notes_for(c, &ws)?));
    m.insert("settings".into(), Value::Object(settings));
    m.insert("history".into(), history_weeks(c)?);
    m.insert("no_school".into(), json!(rows(c, "SELECT * FROM no_school_days ORDER BY day_date", [])?));
    m.insert("leftovers".into(), json!(orphans));
    m.insert("data_dir".into(), json!(data_dir));
    m.insert("can_quit".into(), json!(true));
    m.insert("share_url".into(), json!(share_url));
    m.insert("version".into(), json!(env!("CARGO_PKG_VERSION")));
    Ok(Value::Object(m))
}

// ───────────────────────────────────────────────────────── entry API ──
pub fn create_entry(c: &Connection, b: &Body) -> Res {
    let ws = week_of(b);
    let gid = req_int(b, "grade_id")?;
    let day = as_text(get(b, "day")).filter(|d| day_idx(d).is_some()).ok_or_else(|| bad("bad day"))?;
    let subject = match as_text(get(b, "subject")).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => "Untitled".to_string(),
    };
    let lesson = text_or_empty(b, "lesson_text");
    let sid = opt_id(b, "student_id");
    let slot = match as_int(get(b, "sort_order")) {
        Some(s) if slot_taken(c, gid, &ws, day, s, None)?.is_none() => s,
        _ => free_slot(c, gid, &ws, day, None)?,
    };
    let link = as_text(get(b, "link_id")).filter(|s| !s.is_empty());
    let eid = insert_entry(c, gid, &ws, day, slot, &subject, &lesson, sid, link)?;
    if sid.is_none() && truthy(get(b, "excluded")) {
        set_exclusions(c, eid, &int_list(get(b, "excluded")))?;
    }
    changed(c, &[eid])
}

pub fn update_entry(c: &Connection, eid: i64, b: &Body) -> Res {
    let e = get_entry(c, eid)?;
    let mut ch = vec![eid];
    let link = fl(&e);
    if has(b, "subject") {
        let s = match as_text(get(b, "subject")).map(str::trim) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => fs(&e, "subject").to_string(),
        };
        c.execute("UPDATE schedule_entries SET subject=? WHERE id=?", params![s, eid])?;
    }
    if has(b, "lesson_text") {
        c.execute("UPDATE schedule_entries SET lesson_text=? WHERE id=?", params![text_or_empty(b, "lesson_text"), eid])?;
    }
    let new_sid = opt_id(b, "student_id");
    if has(b, "student_id") {
        c.execute("UPDATE schedule_entries SET student_id=? WHERE id=?", params![new_sid, eid])?;
        if new_sid.is_some() {
            set_exclusions(c, eid, &[])?;
            if link.is_some() {
                // a personal version can't stay linked
                c.execute("UPDATE schedule_entries SET link_id=NULL WHERE id=?", [eid])?;
            }
        }
    }
    let eff_sid = if has(b, "student_id") { new_sid } else { fo(&e, "student_id") };
    if has(b, "excluded") && eff_sid.is_none() {
        set_exclusions(c, eid, &int_list(get(b, "excluded")))?;
    }
    let made_personal = has(b, "student_id") && truthy(get(b, "student_id"));
    // keep linked grades in step
    if link.is_some() && (has(b, "subject") || has(b, "lesson_text")) && !made_personal {
        let cur = one(c, "SELECT subject, lesson_text FROM schedule_entries WHERE id=?", [eid])?.unwrap_or_default();
        for m in link_members(c, link.as_deref(), Some(eid))? {
            c.execute(
                "UPDATE schedule_entries SET subject=?, lesson_text=? WHERE id=?",
                params![fs(&cur, "subject"), fs(&cur, "lesson_text"), fi(&m, "id")],
            )?;
            ch.push(fi(&m, "id"));
        }
    }
    if made_personal && link.is_some() {
        ch.extend(member_ids(c, link.as_deref(), None)?);
        tidy_link(c, link.as_deref())?;
    }
    changed(c, &ch)
}

pub fn delete_entry(c: &Connection, eid: i64, linked: bool) -> Res {
    let e = get_entry(c, eid)?;
    let link = fl(&e);
    let mut del = vec![eid];
    if linked && link.is_some() {
        del.extend(member_ids(c, link.as_deref(), Some(eid))?);
    }
    for i in &del {
        c.execute("DELETE FROM schedule_entries WHERE id=?", [i])?;
        c.execute("DELETE FROM entry_exclusions WHERE entry_id=?", [i])?;
    }
    // (the link id is left on any remaining partner so Undo can re-join them)
    let rest = member_ids(c, link.as_deref(), None)?;
    Ok(json!({ "deleted": del, "changed": entries_by_ids(c, &rest)? }))
}

/// Drag & drop: move to an empty cell, or swap with whatever is there.
pub fn move_entry(c: &Connection, eid: i64, b: &Body) -> Res {
    let e = get_entry(c, eid)?;
    let gid = req_int(b, "grade_id")?;
    let day = as_text(get(b, "day")).filter(|d| day_idx(d).is_some()).ok_or_else(|| bad("bad day"))?;
    let slot = req_int(b, "sort_order")?;
    let other = slot_taken(c, gid, fs(&e, "week_start"), day, slot, Some(eid))?;
    let mut ch = vec![eid];

    let place = |entry: &Row, ngid: i64, nday: &str, nslot: i64, ch: &mut Vec<i64>| -> rusqlite::Result<()> {
        let id = fi(entry, "id");
        c.execute("UPDATE schedule_entries SET grade_id=?, day=?, sort_order=? WHERE id=?", params![ngid, nday, nslot, id])?;
        if ngid != fi(entry, "grade_id") {
            // new grade: personal settings & links don't carry over
            c.execute("UPDATE schedule_entries SET student_id=NULL, link_id=NULL WHERE id=?", [id])?;
            set_exclusions(c, id, &[])?;
            if let Some(l) = fl(entry) {
                ch.extend(member_ids(c, Some(&l), None)?);
                tidy_link(c, Some(&l))?;
            }
        }
        Ok(())
    };
    if let Some(o) = other {
        let oe = get_entry(c, o)?;
        place(&oe, fi(&e, "grade_id"), fs(&e, "day"), fi(&e, "sort_order"), &mut ch)?;
        ch.push(o);
    }
    place(&e, gid, day, slot, &mut ch)?;
    changed(c, &ch)
}

/// Copy (linked or not) into other grades, same day and row if that row is free.
pub fn copy_entry(c: &Connection, eid: i64, b: &Body) -> Res {
    let e = get_entry(c, eid)?;
    let link = if has(b, "link") { truthy(get(b, "link")) } else { true };
    let own = fi(&e, "grade_id");
    let targets: Vec<i64> = int_list(get(b, "grade_ids")).into_iter().filter(|t| *t != own).collect();
    let personal = fo(&e, "student_id").is_some();
    let mut ch = vec![eid];
    let mut lid = fl(&e);
    if link && !targets.is_empty() && lid.is_none() && !personal {
        let l = new_link();
        c.execute("UPDATE schedule_entries SET link_id=? WHERE id=?", params![l, eid])?;
        lid = Some(l);
    }
    let have: HashSet<i64> = if link && lid.is_some() {
        link_members(c, lid.as_deref(), None)?.iter().map(|m| fi(m, "grade_id")).collect()
    } else {
        HashSet::new()
    };
    let ws = fs(&e, "week_start");
    let day = fs(&e, "day");
    for t in targets {
        if have.contains(&t) {
            continue;
        }
        let slot = free_slot(c, t, ws, day, Some(fi(&e, "sort_order")))?;
        let l = if link && !personal { lid.as_deref() } else { None };
        ch.push(insert_entry(c, t, ws, day, slot, fs(&e, "subject"), fs(&e, "lesson_text"), None, l)?);
    }
    changed(c, &ch)
}

/// Link this entry with an existing entry in another grade (that one takes this text).
pub fn link_entry(c: &Connection, eid: i64, b: &Body) -> Res {
    let e = get_entry(c, eid)?;
    let o = get_entry(c, req_int(b, "other_id")?)?;
    if fo(&e, "student_id").is_some() || fo(&o, "student_id").is_some() {
        return Err(bad("personal entries can't be linked"));
    }
    let lid = fl(&e).unwrap_or_else(new_link);
    let old = fl(&o);
    c.execute("UPDATE schedule_entries SET link_id=? WHERE id=?", params![lid, eid])?;
    c.execute(
        "UPDATE schedule_entries SET link_id=?, subject=?, lesson_text=? WHERE id=?",
        params![lid, fs(&e, "subject"), fs(&e, "lesson_text"), fi(&o, "id")],
    )?;
    if old.as_ref().is_some_and(|o| *o != lid) {
        tidy_link(c, old.as_deref())?;
    }
    let mut ch = vec![eid, fi(&o, "id")];
    ch.extend(member_ids(c, Some(&lid), None)?);
    changed(c, &ch)
}

pub fn unlink_entry(c: &Connection, eid: i64) -> Res {
    let e = get_entry(c, eid)?;
    let lid = fl(&e);
    let mut ch = vec![eid];
    ch.extend(member_ids(c, lid.as_deref(), None)?);
    c.execute("UPDATE schedule_entries SET link_id=NULL WHERE id=?", [eid])?;
    tidy_link(c, lid.as_deref())?;
    changed(c, &ch)
}

/// Continue this lesson across the rest of the week in the same row
/// (skipping days off and cells that already have something).
pub fn fill_week(c: &Connection, eid: i64) -> Res {
    let e = get_entry(c, eid)?;
    let p = parse_lesson(fs(&e, "lesson_text"));
    let cal = Calendar::load(c)?;
    let ws = fs(&e, "week_start").to_string();
    let mon = parse_date(&ws).ok_or_else(|| bad("bad week"))?;
    let link = fl(&e);
    let siblings: Vec<i64> =
        if link.is_some() { link_members(c, link.as_deref(), Some(eid))?.iter().map(|m| fi(m, "grade_id")).collect() } else { vec![] };
    let gid = fi(&e, "grade_id");
    let row = fi(&e, "sort_order");
    let subject = fs(&e, "subject");
    let start = day_idx(fs(&e, "day")).ok_or_else(|| bad("bad day"))?;
    let mut ch = Vec::new();
    let mut cur = p.as_ref().map(|p| p.val);
    for (i, day) in DAYS.iter().enumerate().skip(start + 1) {
        let dt = mon + Duration::days(i as i64);
        if cal.start.is_some() && !cal.is_school_day(dt) {
            continue;
        }
        if let Some(t) = slot_taken(c, gid, &ws, day, row, None)? {
            // keep counting from what's already there
            let tr = one(c, "SELECT subject, lesson_text FROM schedule_entries WHERE id=?", [t])?.unwrap_or_default();
            if let (Some(p), Some(tp)) = (&p, parse_lesson(fs(&tr, "lesson_text"))) {
                if norm(fs(&tr, "subject")) == norm(subject) && tp.kind == p.kind {
                    cur = Some(tp.val);
                }
            }
            continue;
        }
        let text = match &p {
            Some(p) => {
                let v = add(p.kind, cur.unwrap_or(p.val), 1);
                cur = Some(v);
                lesson_text_for(&p.pre, &p.post, p.sep, p.kind, v)
            }
            None => String::new(),
        };
        let lid = if siblings.is_empty() { None } else { Some(new_link()) };
        ch.push(insert_entry(c, gid, &ws, day, row, subject, &text, fo(&e, "student_id"), lid.as_deref())?);
        for sg in &siblings {
            let slot = free_slot(c, *sg, &ws, day, Some(row))?;
            ch.push(insert_entry(c, *sg, &ws, day, slot, subject, &text, None, lid.as_deref())?);
        }
    }
    changed(c, &ch)
}

// ─────────────────────────────────────────────────────────────── rows ──
/// Row tools for one grade's grid: insert a blank row, delete a row,
/// swap two rows (all five days at once), or close up gaps.
pub fn row_action(c: &Connection, b: &Body) -> Res {
    let ws = week_of(b);
    let gid = req_int(b, "grade_id")?;
    let at = req_int(b, "at")?;
    match as_text(get(b, "action")).unwrap_or("") {
        "insert" => {
            c.execute(
                "UPDATE schedule_entries SET sort_order=sort_order+1 WHERE grade_id=? AND week_start=? AND sort_order>=?",
                params![gid, ws, at],
            )?;
        }
        "delete" => {
            let del = ids(c, "SELECT id FROM schedule_entries WHERE grade_id=? AND week_start=? AND sort_order=?", params![gid, ws, at])?;
            let mut links: Vec<String> = Vec::new();
            for r in rows(
                c,
                "SELECT link_id FROM schedule_entries WHERE grade_id=? AND week_start=? AND sort_order=? AND link_id IS NOT NULL",
                params![gid, ws, at],
            )? {
                if let Some(l) = fl(&r) {
                    if !links.contains(&l) {
                        links.push(l);
                    }
                }
            }
            for i in del {
                c.execute("DELETE FROM schedule_entries WHERE id=?", [i])?;
                c.execute("DELETE FROM entry_exclusions WHERE entry_id=?", [i])?;
            }
            for l in &links {
                tidy_link(c, Some(l))?;
            }
            c.execute(
                "UPDATE schedule_entries SET sort_order=sort_order-1 WHERE grade_id=? AND week_start=? AND sort_order>?",
                params![gid, ws, at],
            )?;
        }
        "swap" => {
            let to = req_int(b, "to")?;
            c.execute("UPDATE schedule_entries SET sort_order=-999 WHERE grade_id=? AND week_start=? AND sort_order=?", params![gid, ws, at])?;
            c.execute("UPDATE schedule_entries SET sort_order=? WHERE grade_id=? AND week_start=? AND sort_order=?", params![at, gid, ws, to])?;
            c.execute("UPDATE schedule_entries SET sort_order=? WHERE grade_id=? AND week_start=? AND sort_order=-999", params![to, gid, ws])?;
        }
        "compact" => {
            for day in DAYS {
                let list = ids(
                    c,
                    "SELECT id FROM schedule_entries WHERE grade_id=? AND week_start=? AND day=? ORDER BY sort_order,id",
                    params![gid, ws, day],
                )?;
                for (i, x) in list.iter().enumerate() {
                    c.execute("UPDATE schedule_entries SET sort_order=? WHERE id=?", params![i as i64, x])?;
                }
            }
        }
        _ => return Err(bad("unknown action")),
    }
    Ok(json!({ "entries": week_entries(c, &ws)? }))
}

// ─────────────────────────────────────────────────────── week actions ──
/// Build this week from an earlier one. Mode 'advance' moves lesson numbers on
/// and leaves names blank, mode 'copy' copies everything exactly. Only cells
/// that are missing are added; nothing already typed is touched.
pub fn start_week(c: &Connection, b: &Body) -> Res {
    let ws = week_of(b);
    let src = logic::parse_week(as_text(get(b, "source")));
    let advance = match get(b, "mode") {
        Value::Null if !has(b, "mode") => true,
        v => v.as_str() == Some("advance"),
    };
    let parts: Vec<String> = match get(b, "parts") {
        v if truthy(v) => v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        _ => vec!["schedule".into(), "weekly".into(), "notes".into()],
    };
    let part = |p: &str| parts.iter().any(|x| x == p);
    let cal = Calendar::load(c)?;
    let mon = parse_date(&ws).unwrap();
    let student_ids: HashSet<i64> = ids(c, "SELECT id FROM students", [])?.into_iter().collect();
    let grade_ids: HashSet<i64> = ids(c, "SELECT id FROM grades", [])?.into_iter().collect();
    let mut added = 0;

    if part("schedule") {
        let mut src_entries: Vec<Row> = week_entries(c, &src)?
            .into_iter()
            .filter(|e| grade_ids.contains(&fi(e, "grade_id")) && fo(e, "student_id").is_none_or(|s| student_ids.contains(&s)))
            .collect();
        src_entries.sort_by_key(|e| (day_idx(fs(e, "day")).unwrap_or(9), fi(e, "sort_order"), fi(e, "id")));
        // number each subject's lessons as one continuing track
        let mut order: Vec<(i64, String, Option<i64>)> = Vec::new();
        let mut tracks: HashMap<(i64, String, Option<i64>), Vec<usize>> = HashMap::new();
        for (i, e) in src_entries.iter().enumerate() {
            let k = (fi(e, "grade_id"), norm(fs(e, "subject")), fo(e, "student_id"));
            if !tracks.contains_key(&k) {
                order.push(k.clone());
            }
            tracks.entry(k).or_default().push(i);
        }
        let mut new_text: HashMap<i64, String> = HashMap::new();
        for k in &order {
            let members = &tracks[k];
            let texts: Vec<String> = members.iter().map(|&i| fs(&src_entries[i], "lesson_text").to_string()).collect();
            let out = if advance { logic::advance_track(&texts) } else { texts };
            for (&i, t) in members.iter().zip(out) {
                new_text.insert(fi(&src_entries[i], "id"), t);
            }
        }
        // linked entries must stay identical
        let mut link_order: Vec<String> = Vec::new();
        let mut by_link: HashMap<String, Vec<i64>> = HashMap::new();
        for e in &src_entries {
            if let Some(l) = fl(e) {
                if !by_link.contains_key(&l) {
                    link_order.push(l.clone());
                }
                by_link.entry(l).or_default().push(fi(e, "id"));
            }
        }
        for l in &link_order {
            let members = &by_link[l];
            let t = new_text[&members[0]].clone();
            for m in members {
                new_text.insert(*m, t.clone());
            }
        }
        let mut existing: HashSet<(i64, String, String, Option<i64>)> = rows(c, "SELECT * FROM schedule_entries WHERE week_start=?", [&ws])?
            .iter()
            .map(|r| (fi(r, "grade_id"), fs(r, "day").to_string(), norm(fs(r, "subject")), fo(r, "student_id")))
            .collect();
        let mut link_map: Vec<(String, String)> = Vec::new();
        for e in &src_entries {
            let Some(di) = day_idx(fs(e, "day")) else { continue };
            let dt = mon + Duration::days(di as i64);
            if cal.start.is_some() && cal.off.contains(&logic::iso(dt)) {
                continue;
            }
            let key = (fi(e, "grade_id"), fs(e, "day").to_string(), norm(fs(e, "subject")), fo(e, "student_id"));
            if existing.contains(&key) {
                continue;
            }
            let lid = fl(e).map(|old| match link_map.iter().find(|(o, _)| *o == old) {
                Some((_, n)) => n.clone(),
                None => {
                    let n = new_link();
                    link_map.push((old, n.clone()));
                    n
                }
            });
            let slot = free_slot(c, fi(e, "grade_id"), &ws, fs(e, "day"), Some(fi(e, "sort_order")))?;
            let nid = insert_entry(
                c, fi(e, "grade_id"), &ws, fs(e, "day"), slot, fs(e, "subject"), &new_text[&fi(e, "id")],
                fo(e, "student_id"), lid.as_deref(),
            )?;
            if fo(e, "student_id").is_none() {
                let ex: Vec<i64> = int_list(&e["excluded"]).into_iter().filter(|s| student_ids.contains(s)).collect();
                set_exclusions(c, nid, &ex)?;
            }
            existing.insert(key);
            added += 1;
        }
        for (_, l) in &link_map {
            tidy_link(c, Some(l))?;
        }
    }

    if part("weekly") && count(c, "SELECT COUNT(*) FROM weekly_items WHERE week_start=?", [&ws])? == 0 {
        let wsrc = scalar_str(c, "SELECT MAX(week_start) m FROM weekly_items WHERE week_start<=? AND week_start<>?", [&src, &ws])?
            .filter(|s| !s.is_empty())
            .or(scalar_str(c, "SELECT MAX(week_start) m FROM weekly_items WHERE week_start<?", [&ws])?.filter(|s| !s.is_empty()));
        if let Some(wsrc) = wsrc {
            for (i, w) in weekly_for(c, &wsrc)?.iter().enumerate() {
                let desc = if advance { logic::advance_weekly_text(fs(w, "description")) } else { fs(w, "description").to_string() };
                c.execute(
                    "INSERT INTO weekly_items (week_start,subject,description,sort_order) VALUES (?,?,?,?)",
                    params![ws, fs(w, "subject"), desc, i as i64],
                )?;
                let nid = c.last_insert_rowid();
                for g in int_list(&w["grade_ids"]) {
                    c.execute("INSERT OR IGNORE INTO weekly_item_grades VALUES (?,?)", [nid, g])?;
                }
            }
        }
    }

    if part("notes") && !notes_for(c, &ws)?.values().any(|v| !v.as_str().unwrap_or("").trim().is_empty()) {
        let nsrc = scalar_str(c, "SELECT MAX(week_start) m FROM week_notes WHERE week_start<? AND TRIM(note_text)<>''", [&ws])?;
        if let Some(nsrc) = nsrc.filter(|s| !s.is_empty()) {
            for (d, t) in notes_for(c, &nsrc)? {
                c.execute(
                    "INSERT OR REPLACE INTO week_notes (week_start,day,note_text) VALUES (?,?,?)",
                    params![ws, d, t.as_str().unwrap_or("")],
                )?;
            }
        }
    }
    Ok(json!({ "added": added }))
}

pub fn clear_week(c: &Connection, b: &Body) -> Res {
    let ws = week_of(b);
    let del = match opt_id(b, "grade_id") {
        Some(g) => ids(c, "SELECT id FROM schedule_entries WHERE week_start=? AND grade_id=?", params![ws, g])?,
        None => ids(c, "SELECT id FROM schedule_entries WHERE week_start=?", [&ws])?,
    };
    for i in &del {
        c.execute("DELETE FROM entry_exclusions WHERE entry_id=?", [i])?;
        c.execute("DELETE FROM schedule_entries WHERE id=?", [i])?;
    }
    for r in rows(c, "SELECT DISTINCT link_id FROM schedule_entries WHERE week_start=? AND link_id IS NOT NULL", [&ws])? {
        tidy_link(c, fl(&r).as_deref())?;
    }
    Ok(json!({ "deleted": del.len() }))
}

pub fn autolink(c: &Connection, b: &Body) -> Res {
    Ok(json!({ "linked": autolink_week(c, &week_of(b))? }))
}

// ──────────────────────────────────────────────────── weekly & notes ──
pub fn weekly_create(c: &Connection, b: &Body) -> Res {
    let ws = week_of(b);
    let n = count(c, "SELECT COUNT(*) n FROM weekly_items WHERE week_start=?", [&ws])?;
    let subject = match as_text(get(b, "subject")).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => "Untitled".into(),
    };
    c.execute(
        "INSERT INTO weekly_items (week_start,subject,description,sort_order) VALUES (?,?,?,?)",
        params![ws, subject, text_or_empty(b, "description"), n],
    )?;
    let wid = c.last_insert_rowid();
    for g in int_list(get(b, "grade_ids")) {
        c.execute("INSERT OR IGNORE INTO weekly_item_grades VALUES (?,?)", [wid, g])?;
    }
    Ok(json!({ "weekly": weekly_for(c, &ws)? }))
}

fn move_in(list: &mut [i64], id: i64, by: Option<i64>) {
    let (Some(i), Some(by)) = (list.iter().position(|x| *x == id), by) else { return };
    let j = i as i64 + by;
    if j >= 0 && (j as usize) < list.len() {
        list.swap(i, j as usize);
    }
}

pub fn weekly_update(c: &Connection, wid: i64, b: &Body) -> Res {
    let w = one(c, "SELECT * FROM weekly_items WHERE id=?", [wid])?.ok_or_else(not_found)?;
    let ws = fs(&w, "week_start").to_string();
    if has(b, "subject") {
        let s = match as_text(get(b, "subject")).map(str::trim) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => fs(&w, "subject").to_string(),
        };
        c.execute("UPDATE weekly_items SET subject=? WHERE id=?", params![s, wid])?;
    }
    if has(b, "description") {
        c.execute("UPDATE weekly_items SET description=? WHERE id=?", params![text_or_empty(b, "description"), wid])?;
    }
    if has(b, "grade_ids") {
        c.execute("DELETE FROM weekly_item_grades WHERE item_id=?", [wid])?;
        for g in int_list(get(b, "grade_ids")) {
            c.execute("INSERT OR IGNORE INTO weekly_item_grades VALUES (?,?)", [wid, g])?;
        }
    }
    if has(b, "move") {
        let mut items = ids(c, "SELECT id FROM weekly_items WHERE week_start=? ORDER BY sort_order,id", [&ws])?;
        move_in(&mut items, wid, as_int(get(b, "move")));
        for (k, x) in items.iter().enumerate() {
            c.execute("UPDATE weekly_items SET sort_order=? WHERE id=?", params![k as i64, x])?;
        }
    }
    Ok(json!({ "weekly": weekly_for(c, &ws)? }))
}

pub fn weekly_delete(c: &Connection, wid: i64) -> Res {
    let w = one(c, "SELECT * FROM weekly_items WHERE id=?", [wid])?.ok_or_else(not_found)?;
    c.execute("DELETE FROM weekly_items WHERE id=?", [wid])?;
    c.execute("DELETE FROM weekly_item_grades WHERE item_id=?", [wid])?;
    Ok(json!({ "weekly": weekly_for(c, fs(&w, "week_start"))? }))
}

pub fn put_note(c: &Connection, b: &Body) -> Res {
    let ws = week_of(b);
    let day = as_text(get(b, "day")).filter(|d| day_idx(d).is_some()).ok_or_else(|| bad("bad day"))?;
    c.execute(
        "INSERT OR REPLACE INTO week_notes (week_start,day,note_text) VALUES (?,?,?)",
        params![ws, day, text_or_empty(b, "text")],
    )?;
    Ok(json!({ "ok": true }))
}

// ──────────────────────────────────────── grades, students, settings ──
fn ok() -> Res {
    Ok(json!({ "ok": true }))
}

pub fn grade_create(c: &Connection, b: &Body) -> Res {
    let name = as_text(get(b, "name")).map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Err(bad("name is missing"));
    }
    let n = count(c, "SELECT COALESCE(MAX(sort_order),-1)+1 n FROM grades", [])?;
    let color = as_text(get(b, "color")).filter(|s| !s.is_empty()).unwrap_or("#1e3054");
    c.execute("INSERT INTO grades (name,color,sort_order) VALUES (?,?,?)", params![name, color, n])?;
    ok()
}

pub fn grade_update(c: &Connection, gid: i64, b: &Body) -> Res {
    if let Some(n) = as_text(get(b, "name")).map(str::trim).filter(|s| !s.is_empty()) {
        c.execute("UPDATE grades SET name=? WHERE id=?", params![n, gid])?;
    }
    if let Some(col) = as_text(get(b, "color")).filter(|s| !s.is_empty()) {
        c.execute("UPDATE grades SET color=? WHERE id=?", params![col, gid])?;
    }
    if has(b, "move") {
        let mut list = ids(c, "SELECT id FROM grades ORDER BY sort_order,id", [])?;
        if !list.contains(&gid) {
            return Err(not_found());
        }
        move_in(&mut list, gid, as_int(get(b, "move")));
        for (k, x) in list.iter().enumerate() {
            c.execute("UPDATE grades SET sort_order=? WHERE id=?", params![k as i64, x])?;
        }
    }
    ok()
}

pub fn grade_delete(c: &Connection, gid: i64) -> Res {
    let del = ids(c, "SELECT id FROM schedule_entries WHERE grade_id=?", [gid])?;
    let links: Vec<String> =
        rows(c, "SELECT DISTINCT link_id FROM schedule_entries WHERE grade_id=? AND link_id IS NOT NULL", [gid])?.iter().filter_map(fl).collect();
    for i in del {
        c.execute("DELETE FROM entry_exclusions WHERE entry_id=?", [i])?;
    }
    for s in ids(c, "SELECT id FROM students WHERE grade_id=?", [gid])? {
        c.execute("DELETE FROM entry_exclusions WHERE student_id=?", [s])?;
    }
    for (t, col) in [
        ("schedule_entries", "grade_id"),
        ("year_template", "grade_id"),
        ("weekly_assignment_grades", "grade_id"),
        ("weekly_item_grades", "grade_id"),
        ("students", "grade_id"),
        ("grades", "id"),
    ] {
        c.execute(&format!("DELETE FROM {t} WHERE {col}=?"), [gid])?;
    }
    for l in &links {
        tidy_link(c, Some(l))?;
    }
    ok()
}

pub fn student_create(c: &Connection, b: &Body) -> Res {
    let name = as_text(get(b, "name")).map(str::trim).unwrap_or("");
    let gid = opt_id(b, "grade_id");
    let (false, Some(gid)) = (name.is_empty(), gid) else { return Err(bad("name and grade are needed")) };
    let n = count(c, "SELECT COALESCE(MAX(sort_order),-1)+1 n FROM students WHERE grade_id=?", [gid])?;
    c.execute("INSERT INTO students (name,grade_id,sort_order) VALUES (?,?,?)", params![name, gid, n])?;
    ok()
}

pub fn student_update(c: &Connection, sid: i64, b: &Body) -> Res {
    let s = one(c, "SELECT * FROM students WHERE id=?", [sid])?.ok_or_else(not_found)?;
    if let Some(n) = as_text(get(b, "name")).map(str::trim).filter(|s| !s.is_empty()) {
        c.execute("UPDATE students SET name=? WHERE id=?", params![n, sid])?;
    }
    if let Some(g) = opt_id(b, "grade_id").filter(|g| *g != fi(&s, "grade_id")) {
        let n = count(c, "SELECT COALESCE(MAX(sort_order),-1)+1 n FROM students WHERE grade_id=?", [g])?;
        c.execute("UPDATE students SET grade_id=?, sort_order=? WHERE id=?", params![g, n, sid])?;
    }
    if has(b, "move") {
        let mut list = ids(c, "SELECT id FROM students WHERE grade_id=? ORDER BY sort_order,name", [fi(&s, "grade_id")])?;
        if list.contains(&sid) {
            move_in(&mut list, sid, as_int(get(b, "move")));
            for (k, x) in list.iter().enumerate() {
                c.execute("UPDATE students SET sort_order=? WHERE id=?", params![k as i64, x])?;
            }
        }
    }
    ok()
}

pub fn student_delete(c: &Connection, sid: i64) -> Res {
    c.execute("DELETE FROM entry_exclusions WHERE student_id=?", [sid])?;
    c.execute("DELETE FROM entry_exclusions WHERE entry_id IN (SELECT id FROM schedule_entries WHERE student_id=?)", [sid])?;
    c.execute("DELETE FROM schedule_entries WHERE student_id=?", [sid])?;
    c.execute("DELETE FROM students WHERE id=?", [sid])?;
    ok()
}

/// Remove 'only for' entries and 'leave out' marks for students that no longer exist.
pub fn cleanup(c: &Connection) -> Res {
    let n = c.execute(
        "DELETE FROM entry_exclusions WHERE student_id NOT IN (SELECT id FROM students) \
         OR entry_id NOT IN (SELECT id FROM schedule_entries)",
        [],
    )? + c.execute(
        "DELETE FROM schedule_entries WHERE student_id IS NOT NULL AND student_id NOT IN (SELECT id FROM students)",
        [],
    )?;
    Ok(json!({ "removed": n }))
}

pub fn put_settings(c: &Connection, b: &Body) -> Res {
    for (k, v) in b {
        if db::editable_setting(k) {
            let s = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            c.execute("INSERT OR REPLACE INTO settings (key,value) VALUES (?,?)", params![k, s])?;
        }
    }
    Ok(json!({ "settings": all_settings(c)? }))
}

pub fn noschool_add(c: &Connection, b: &Body) -> Res {
    let a = as_text(get(b, "from")).and_then(parse_date).ok_or_else(|| bad("from date is missing"))?;
    let z = as_text(get(b, "to")).and_then(parse_date).unwrap_or(a);
    let (a, z) = if z < a { (z, a) } else { (a, z) };
    let label = as_text(get(b, "label")).map(str::trim).unwrap_or("");
    let mut cur = a;
    while cur <= z {
        if logic::weekday(cur) < 5 {
            c.execute("INSERT OR REPLACE INTO no_school_days (day_date,label) VALUES (?,?)", params![logic::iso(cur), label])?;
        }
        cur += Duration::days(1);
    }
    ok()
}

pub fn noschool_delete(c: &Connection, day: &str) -> Res {
    c.execute("DELETE FROM no_school_days WHERE day_date=?", [day])?;
    ok()
}
