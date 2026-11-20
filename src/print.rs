//! The three printouts, rendered from the same Jinja templates as the Python app.

use crate::api::{week_entries, weekly_for, notes_for, ApiErr};
use crate::db::{all_settings, one, rows, Row, PRINT_KEYS};
use crate::logic::{self, print_lesson, Calendar, Highlighter, DAYS};
use minijinja::{AutoEscape, Environment, Output, State, Value as JV};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

static ENV: LazyLock<Environment<'static>> = LazyLock::new(|| {
    let mut e = Environment::new();
    e.set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
    e.set_formatter(markupsafe_formatter);
    e.add_template("print_teacher.html", include_str!("../assets/templates/print_teacher.html")).unwrap();
    e.add_template("print_student.html", include_str!("../assets/templates/print_student.html")).unwrap();
    e.add_template("print_bulk.html", include_str!("../assets/templates/print_bulk.html")).unwrap();
    e
});

/// Escape exactly like Jinja/MarkupSafe does in the Python app.
fn markupsafe(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&#34;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

fn markupsafe_formatter(out: &mut Output, state: &State, value: &JV) -> Result<(), minijinja::Error> {
    if state.auto_escape() != AutoEscape::None && !value.is_safe() {
        let s = if value.is_undefined() || value.is_none() { String::new() } else { value.to_string() };
        out.write_str(&markupsafe(&s)).map_err(|_| minijinja::Error::new(minijinja::ErrorKind::WriteFailure, "write failed"))
    } else {
        minijinja::escape_formatter(out, state, value)
    }
}

type Grid = BTreeMap<i64, Map<String, Value>>;

struct Ctx {
    grades: Vec<Row>,
    students: Vec<Row>,
    grid: Grid,
    weekly: Vec<Row>,
    common: BTreeMap<String, JV>,
    teacher_notes: Map<String, Value>,
}

fn id(r: &Row, k: &str) -> i64 {
    r.get(k).and_then(|v| v.as_i64()).unwrap_or(0)
}

fn build(c: &Connection, ws: &str) -> Result<Ctx, ApiErr> {
    let s = all_settings(c)?;
    let grades = rows(c, "SELECT * FROM grades ORDER BY sort_order,id", [])?;
    let students = rows(c, "SELECT * FROM students WHERE active=1 ORDER BY sort_order,name", [])?;
    let hl = Highlighter::new(s.get("test_highlight_keywords").and_then(|v| v.as_str()).unwrap_or(""));
    let names: HashMap<i64, String> =
        students.iter().map(|st| (id(st, "id"), st["name"].as_str().unwrap_or("").to_string())).collect();
    let known: HashSet<i64> = crate::db::ids(c, "SELECT id FROM students", [])?.into_iter().collect();
    let mut grid: Grid = BTreeMap::new();
    for g in &grades {
        let mut days = Map::new();
        for d in DAYS {
            days.insert(d.to_string(), json!([]));
        }
        grid.insert(id(g, "id"), days);
    }
    for mut e in week_entries(c, ws)? {
        let Some(cell) = grid.get_mut(&id(&e, "grade_id")) else { continue };
        let sid = id(&e, "student_id");
        if sid != 0 && !known.contains(&sid) {
            continue; // left over from a removed student
        }
        let lesson = print_lesson(e["lesson_text"].as_str().unwrap_or(""));
        let hit = hl.hit(e["subject"].as_str().unwrap_or(""), &lesson);
        e.insert("lesson_text".into(), json!(lesson));
        e.insert("highlighted".into(), json!(hit));
        e.insert("student_name".into(), if sid != 0 { json!(names.get(&sid)) } else { Value::Null });
        let ex: Vec<&String> =
            e["excluded"].as_array().into_iter().flatten().filter_map(|v| v.as_i64()).filter_map(|i| names.get(&i)).collect();
        e.insert("excluded_names".into(), json!(ex));
        let day = e["day"].as_str().unwrap_or("").to_string();
        if let Some(Value::Array(list)) = cell.get_mut(&day) {
            list.push(Value::Object(e));
        }
    }
    let weekly = weekly_for(c, ws)?;
    let notes = notes_for(c, ws)?;
    let mut teacher_notes = Map::new();
    for d in DAYS {
        teacher_notes.insert(d.to_string(), notes.get(d).cloned().unwrap_or(json!("")));
    }

    let mut common: BTreeMap<String, JV> = BTreeMap::new();
    for k in PRINT_KEYS {
        common.insert(k.to_string(), JV::from_serialize(s.get(k).cloned().unwrap_or(json!(""))));
    }
    let cal = Calendar::load(c)?;
    let info = logic::week_info(ws, &cal);
    for k in ["week_num", "day_num", "today_str", "week_dates", "week_school_days"] {
        common.insert(k.to_string(), JV::from_serialize(&info[k]));
    }
    common.insert("days".into(), JV::from_serialize(DAYS));
    Ok(Ctx { grades, students, grid, weekly, common, teacher_notes })
}

/// Drop other students' versions and entries this student is left out of;
/// their own version replaces the class version of the same subject.
fn resolve_for_student(day_entries: &Value, sid: i64) -> Value {
    let list = day_entries.as_array().cloned().unwrap_or_default();
    let subj = |e: &Value| e["subject"].as_str().unwrap_or("").trim().to_lowercase();
    let mine: HashSet<String> = list.iter().filter(|e| e["student_id"].as_i64() == Some(sid)).map(subj).collect();
    let out: Vec<Value> = list
        .iter()
        .filter(|e| {
            let esid = e["student_id"].as_i64().filter(|s| *s != 0);
            if esid.is_some_and(|s| s != sid) {
                return false;
            }
            if e["excluded"].as_array().is_some_and(|a| a.iter().any(|x| x.as_i64() == Some(sid))) {
                return false;
            }
            !(esid.is_none() && mine.contains(&subj(e)))
        })
        .cloned()
        .collect();
    Value::Array(out)
}

fn render(name: &str, ctx: BTreeMap<String, JV>) -> Result<String, ApiErr> {
    let t = ENV.get_template(name).map_err(|e| ApiErr(500, e.to_string()))?;
    t.render(&ctx).map_err(|e| ApiErr(500, format!("{e:#}")))
}

fn student_schedule(ctx: &Ctx, gid: i64, sid: i64) -> Map<String, Value> {
    let mut out = Map::new();
    for d in DAYS {
        let raw = ctx.grid.get(&gid).and_then(|g| g.get(d)).cloned().unwrap_or(json!([]));
        out.insert(d.to_string(), resolve_for_student(&raw, sid));
    }
    out
}

fn weekly_for_grade(ctx: &Ctx, gid: i64) -> Vec<&Row> {
    ctx.weekly
        .iter()
        .filter(|w| w["grade_ids"].as_array().is_some_and(|a| a.iter().any(|x| x.as_i64() == Some(gid))))
        .collect()
}

pub fn teacher(c: &Connection, ws: &str) -> Result<String, ApiErr> {
    let ctx = build(c, ws)?;
    let weekly_grades: BTreeMap<i64, Vec<i64>> = ctx
        .weekly
        .iter()
        .map(|w| (id(w, "id"), w["grade_ids"].as_array().into_iter().flatten().filter_map(|v| v.as_i64()).collect()))
        .collect();
    let mut m = ctx.common.clone();
    m.insert("grades".into(), JV::from_serialize(&ctx.grades));
    m.insert("students".into(), JV::from_serialize(&ctx.students));
    m.insert("schedule".into(), JV::from_serialize(&ctx.grid));
    m.insert("weekly".into(), JV::from_serialize(&ctx.weekly));
    m.insert("weekly_grades".into(), JV::from_serialize(&weekly_grades));
    m.insert("teacher_notes".into(), JV::from_serialize(&ctx.teacher_notes));
    render("print_teacher.html", m)
}

/// None if there is no such student.
pub fn student(c: &Connection, ws: &str, sid: i64) -> Result<Option<String>, ApiErr> {
    let ctx = build(c, ws)?;
    let Some(st) = one(c, "SELECT * FROM students WHERE id=?", [sid])? else { return Ok(None) };
    let gid = id(&st, "grade_id");
    let grade = ctx.grades.iter().find(|g| id(g, "id") == gid);
    let mut m = ctx.common.clone();
    m.insert("student".into(), JV::from_serialize(&st));
    m.insert("grade".into(), JV::from_serialize(grade));
    m.insert("grade_schedule".into(), JV::from_serialize(student_schedule(&ctx, gid, sid)));
    m.insert("grade_weekly".into(), JV::from_serialize(weekly_for_grade(&ctx, gid)));
    render("print_student.html", m).map(Some)
}

pub fn bulk(c: &Connection, ws: &str, only_grade: Option<i64>) -> Result<String, ApiErr> {
    let ctx = build(c, ws)?;
    let order: HashMap<i64, usize> = ctx.grades.iter().enumerate().map(|(i, g)| (id(g, "id"), i)).collect();
    let mut studs: Vec<&Row> = ctx.students.iter().collect();
    studs.sort_by(|a, b| {
        let ka = (order.get(&id(a, "grade_id")).copied().unwrap_or(99), a["name"].as_str().unwrap_or(""));
        let kb = (order.get(&id(b, "grade_id")).copied().unwrap_or(99), b["name"].as_str().unwrap_or(""));
        ka.cmp(&kb)
    });
    if let Some(g) = only_grade.filter(|g| *g != 0) {
        studs.retain(|s| id(s, "grade_id") == g);
    }
    let sdata: Vec<Value> = studs
        .iter()
        .map(|s| {
            let gid = id(s, "grade_id");
            json!({
                "student": s,
                "grade": ctx.grades.iter().find(|g| id(g, "id") == gid),
                "schedule": student_schedule(&ctx, gid, id(s, "id")),
                "weekly": weekly_for_grade(&ctx, gid),
            })
        })
        .collect();
    let mut m = ctx.common.clone();
    m.insert("student_data".into(), JV::from_serialize(&sdata));
    render("print_bulk.html", m)
}
