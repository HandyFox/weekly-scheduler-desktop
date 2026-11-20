//! Dates, school-day numbering and lesson-number advancing.
//! These are line-for-line ports of the Python app so both give the same results.

use chrono::{Datelike, Duration, Local, NaiveDate};
use regex::Regex;
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub const DAYS: [&str; 5] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];

pub fn day_idx(day: &str) -> Option<usize> {
    DAYS.iter().position(|d| *d == day)
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// First 10 characters as YYYY-MM-DD, like `date.fromisoformat(s.strip()[:10])`.
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    let head: String = s.chars().take(10).collect();
    if head.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(&head, "%Y-%m-%d").ok()
}

pub fn iso(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub fn weekday(d: NaiveDate) -> i64 {
    d.weekday().num_days_from_monday() as i64
}

pub fn monday(d: NaiveDate) -> NaiveDate {
    d - Duration::days(weekday(d))
}

pub fn monday_of_today() -> String {
    iso(monday(today()))
}

/// Week parameter → that week's Monday (this week if missing or not a date).
pub fn parse_week(param: Option<&str>) -> String {
    match param.and_then(parse_date) {
        Some(d) => iso(monday(d)),
        None => monday_of_today(),
    }
}

pub fn fmt_week_range(ws: &str) -> String {
    let Some(mon) = parse_date(ws) else { return ws.to_string() };
    let fri = mon + Duration::days(4);
    if mon.month() == fri.month() {
        format!("{} {}–{}, {}", mon.format("%b"), mon.day(), fri.day(), mon.year())
    } else {
        format!("{} {} – {} {}, {}", mon.format("%b"), mon.day(), fri.format("%b"), fri.day(), fri.year())
    }
}

pub fn fmt_date_short(d: NaiveDate) -> String {
    format!("{} {}", d.format("%b"), d.day())
}

pub fn fmt_date_long(d: NaiveDate) -> String {
    format!("{}, {} {}, {}", d.format("%A"), d.format("%B"), d.day(), d.year())
}

/// `int(str(v).strip())` with a default.
pub fn loose_int(s: &str, default: i64) -> i64 {
    s.trim().parse::<i64>().unwrap_or(default)
}

// ───────────────────────────────────────────────────────────── calendar ──
/// School-day and school-week numbering, skipping days off.
pub struct Calendar {
    pub start: Option<NaiveDate>,
    pub off: HashSet<String>,
    day_adj: i64,
    week_adj: i64,
}

impl Calendar {
    pub fn load(c: &Connection) -> rusqlite::Result<Calendar> {
        let start = parse_date(&crate::db::setting(c, "school_start_date")?);
        let mut off = HashSet::new();
        let mut st = c.prepare("SELECT day_date FROM no_school_days")?;
        let mut q = st.query([])?;
        while let Some(r) = q.next()? {
            if let Ok(s) = r.get::<_, String>(0) {
                off.insert(s);
            }
        }
        Ok(Calendar {
            start,
            off,
            day_adj: loose_int(&crate::db::setting(c, "day_adjust")?, 0),
            week_adj: loose_int(&crate::db::setting(c, "week_adjust")?, 0),
        })
    }

    pub fn is_school_day(&self, d: NaiveDate) -> bool {
        match self.start {
            Some(s) => d >= s && weekday(d) < 5 && !self.off.contains(&iso(d)),
            None => false,
        }
    }

    pub fn day_number(&self, d: NaiveDate) -> Option<i64> {
        if !self.is_school_day(d) {
            return None;
        }
        let mut n = 0;
        let mut cur = self.start?;
        while cur <= d {
            if self.is_school_day(cur) {
                n += 1;
            }
            cur += Duration::days(1);
        }
        n += self.day_adj;
        (n > 0).then_some(n)
    }

    pub fn week_number(&self, mon: NaiveDate) -> Option<i64> {
        let start = self.start?;
        if !(0..5).any(|i| self.is_school_day(mon + Duration::days(i))) {
            return None;
        }
        let mut n = 0;
        let mut cur = start - Duration::days(weekday(start));
        while cur <= mon {
            if (0..5).any(|i| self.is_school_day(cur + Duration::days(i))) {
                n += 1;
            }
            cur += Duration::days(7);
        }
        n += self.week_adj;
        (n > 0).then_some(n)
    }
}

pub fn week_info(ws: &str, cal: &Calendar) -> Map<String, Value> {
    let mon = parse_date(ws).unwrap_or_else(|| monday(today()));
    let t = today();
    let mut dates = Map::new();
    let mut isos = Map::new();
    let mut school = Map::new();
    let mut off = Map::new();
    for (i, d) in DAYS.iter().enumerate() {
        let v = mon + Duration::days(i as i64);
        dates.insert(d.to_string(), json!(fmt_date_short(v)));
        isos.insert(d.to_string(), json!(iso(v)));
        if let Some(n) = cal.day_number(v) {
            school.insert(d.to_string(), json!(n));
        }
        off.insert(d.to_string(), json!(cal.off.contains(&iso(v))));
    }
    let mut m = Map::new();
    m.insert("week_num".into(), json!(cal.week_number(mon)));
    m.insert("day_num".into(), json!(cal.day_number(t)));
    m.insert("today_str".into(), json!(fmt_date_long(t)));
    m.insert("week_dates".into(), Value::Object(dates));
    m.insert("week_iso".into(), Value::Object(isos));
    m.insert("week_school_days".into(), Value::Object(school));
    m.insert("off_days".into(), Value::Object(off));
    m
}

// ─────────────────────────────────────────────────────── lesson numbers ──
// Lessons look like "Lesson 12 ~ Adding Fractions", "3.4 ~ Factoring",
// "Worksheet 8 ~ ...", "Group F", "Lesson 3". The part before "~" holds the
// number; the part after is the lesson name.
static HEAD_NUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<pre>\D*?)(?P<num>\d+(?:\.\d+)?)(?P<post>.*)$").unwrap());
static HEAD_LET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?P<pre>.*\s)(?P<num>[A-Z])$").unwrap());
static WEEKLY_NUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b((?:Lesson|Unit|Week)\s+)(\d+)\b").unwrap());
static NEEDS_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"~\s*$").unwrap());
static TRAILING_SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*~\s*$").unwrap());

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Int,
    Dot,
    Let,
}

/// (whole, part): "12" → (12, 0); "3.4" → (3, 4); "F" → (70, 0)
pub type Val = (i64, i64);

#[derive(Clone, Debug)]
pub struct Parsed {
    pub pre: String,
    pub post: String,
    pub kind: Kind,
    pub val: Val,
    pub sep: bool,
}

pub fn split_lesson(text: &str) -> (String, String, bool) {
    match text.find('~') {
        Some(i) => (text[..i].trim().to_string(), text[i + 1..].trim().to_string(), true),
        None => (text.trim().to_string(), String::new(), false),
    }
}

pub fn parse_lesson(text: &str) -> Option<Parsed> {
    let (head, _title, sep) = split_lesson(text);
    if head.is_empty() {
        return None;
    }
    if let Some(m) = HEAD_NUM.captures(&head) {
        let num = &m["num"];
        let (kind, val) = match num.split_once('.') {
            Some((a, b)) => (Kind::Dot, (a.parse().ok()?, b.parse().ok()?)),
            None => (Kind::Int, (num.parse().ok()?, 0)),
        };
        return Some(Parsed { pre: m["pre"].to_string(), post: m["post"].to_string(), kind, val, sep });
    }
    if let Some(m) = HEAD_LET.captures(&head) {
        let c = m["num"].chars().next()? as i64;
        return Some(Parsed { pre: m["pre"].to_string(), post: String::new(), kind: Kind::Let, val: (c, 0), sep });
    }
    None
}

fn fmt_val(kind: Kind, v: Val) -> String {
    match kind {
        Kind::Dot => format!("{}.{}", v.0, v.1),
        Kind::Let => char::from_u32(v.0.clamp(65, 90) as u32).unwrap_or('A').to_string(),
        Kind::Int => v.0.to_string(),
    }
}

/// How far the source moved from a to b (0 = same lesson repeated).
fn step(kind: Kind, a: Val, b: Val) -> i64 {
    let d = if kind == Kind::Dot {
        if a.0 != b.0 {
            return 1;
        }
        b.1 - a.1
    } else {
        b.0 - a.0
    };
    if (0..=3).contains(&d) {
        d
    } else {
        1
    }
}

pub fn add(kind: Kind, v: Val, n: i64) -> Val {
    if kind == Kind::Dot {
        (v.0, v.1 + n)
    } else {
        (v.0 + n, 0)
    }
}

/// The lesson text with a new number and the name left blank.
pub fn lesson_text_for(pre: &str, post: &str, sep: bool, kind: Kind, val: Val) -> String {
    let head = format!("{pre}{}{post}", fmt_val(kind, val));
    if sep {
        format!("{head} ~ ")
    } else {
        head
    }
}

/// One subject's lesson texts for a week (in day order) → the texts for the
/// following week: numbers continue, lesson names left blank. Subjects
/// without numbers are copied unchanged.
pub fn advance_track(texts: &[String]) -> Vec<String> {
    let parsed: Vec<Option<Parsed>> = texts.iter().map(|t| parse_lesson(t)).collect();
    // most common (pre, post, kind); ties go to the first one seen
    let mut order: Vec<(String, String, Kind)> = Vec::new();
    let mut count: HashMap<(String, String, Kind), usize> = HashMap::new();
    for p in parsed.iter().flatten() {
        let k = (p.pre.clone(), p.post.clone(), p.kind);
        if !count.contains_key(&k) {
            order.push(k.clone());
        }
        *count.entry(k).or_default() += 1;
    }
    if order.is_empty() {
        return texts.to_vec();
    }
    let mut tmpl = &order[0];
    for k in &order {
        if count[k] > count[tmpl] {
            tmpl = k;
        }
    }
    let numbered: Vec<Option<&Parsed>> = parsed
        .iter()
        .map(|p| p.as_ref().filter(|p| p.pre == tmpl.0 && p.post == tmpl.1 && p.kind == tmpl.2))
        .collect();
    let vals: Vec<Val> = numbered.iter().flatten().map(|p| p.val).collect();
    // Same number every day (e.g. a workbook "1205" used all week) → keep it.
    if vals.len() >= 3 && vals.iter().all(|v| *v == vals[0]) {
        return texts.to_vec();
    }
    let kind = tmpl.2;
    let sep = parsed.iter().flatten().any(|p| p.sep);
    let mut out = Vec::new();
    let mut cur: Option<Val> = None;
    let mut prev: Option<Val> = None;
    for p in &numbered {
        let next = match cur {
            None => add(kind, *vals.last().unwrap(), 1),
            Some(c) => {
                let n = match (p, prev) {
                    (Some(p), Some(pv)) => step(kind, pv, p.val),
                    _ => 1,
                };
                add(kind, c, n)
            }
        };
        cur = Some(next);
        prev = p.map(|p| p.val);
        out.push(lesson_text_for(&tmpl.0, &tmpl.1, sep, kind, next));
    }
    out
}

/// Weekly items only move a 'Lesson N' / 'Unit N' / 'Week N' number.
pub fn advance_weekly_text(text: &str) -> String {
    WEEKLY_NUM
        .replacen(text, 1, |c: &regex::Captures| match c[2].parse::<u128>() {
            Ok(n) => format!("{}{}", &c[1], n + 1),
            Err(_) => c[0].to_string(),
        })
        .into_owned()
}

pub fn needs_name(text: &str) -> bool {
    NEEDS_NAME.is_match(text)
}

/// Hide an unfinished ' ~ ' at the end so a blank name never prints.
pub fn print_lesson(text: &str) -> String {
    TRAILING_SEP.replace(text, "").into_owned()
}

/// Test highlighting: any keyword as a whole word in "subject lesson".
pub struct Highlighter(Vec<Regex>);

impl Highlighter {
    pub fn new(keywords: &str) -> Self {
        Highlighter(
            keywords
                .split(',')
                .map(|k| k.trim().to_lowercase())
                .filter(|k| !k.is_empty())
                .filter_map(|k| Regex::new(&format!(r"\b{}\b", regex::escape(&k))).ok())
                .collect(),
        )
    }
    pub fn hit(&self, subject: &str, lesson: &str) -> bool {
        let s = format!("{subject} {lesson}").to_lowercase();
        self.0.iter().any(|p| p.is_match(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lessons_continue() {
        let out = advance_track(&v(&["Lesson 32 ~ A", "Lesson 33 ~ B", "Lesson 34 ~ C", "Lesson 35 ~ D", "Lesson 36 ~ E"]));
        assert_eq!(out, v(&["Lesson 37 ~ ", "Lesson 38 ~ ", "Lesson 39 ~ ", "Lesson 40 ~ ", "Lesson 41 ~ "]));
    }
    #[test]
    fn dotted_numbers() {
        let out = advance_track(&v(&["2.16 ~ x", "3.1 ~ a", "3.2 ~ b", "3.3 ~ c", "3.4 ~ d"]));
        assert_eq!(out, v(&["3.5 ~ ", "3.6 ~ ", "3.7 ~ ", "3.8 ~ ", "3.9 ~ "]));
        assert_eq!(advance_track(&v(&["3.9 ~ x"])), v(&["3.10 ~ "]));
    }
    #[test]
    fn repeats_and_constants() {
        assert_eq!(advance_track(&v(&["Lesson 6", "Lesson 6"])), v(&["Lesson 7", "Lesson 7"]));
        let same = v(&["1205", "1205", "1205"]);
        assert_eq!(advance_track(&same), same);
    }
    #[test]
    fn letters_and_odd_ones() {
        assert_eq!(advance_track(&v(&["Group F", "Group G"])), v(&["Group H", "Group I"]));
        let out = advance_track(&v(&["Lesson 31", "Lesson 32", "Chapter 3 Test", "Lesson 34", "Lesson 35"]));
        assert_eq!(out, v(&["Lesson 36", "Lesson 37", "Lesson 38", "Lesson 39", "Lesson 40"]));
        let plain = v(&["Read", "Read"]);
        assert_eq!(advance_track(&plain), plain);
    }
    #[test]
    fn weekly_and_print() {
        assert_eq!(advance_weekly_text("Spelling Unit 4 words"), "Spelling Unit 5 words");
        assert_eq!(print_lesson("Lesson 4 ~ "), "Lesson 4");
        assert!(needs_name("Lesson 4 ~ "));
        assert!(!needs_name("Lesson 4 ~ Fractions"));
    }
    #[test]
    fn dates() {
        assert_eq!(fmt_week_range("2026-11-30"), "Nov 30 – Dec 4, 2026");
        assert_eq!(fmt_week_range("2026-11-16"), "Nov 16–20, 2026");
        assert_eq!(fmt_date_long(NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()), "Saturday, November 14, 2026");
        assert_eq!(parse_week(Some("2026-11-19")), "2026-11-16");
    }
    #[test]
    fn highlight() {
        let h = Highlighter::new("test, quiz");
        assert!(h.hit("Math", "Chapter 3 Test"));
        assert!(!h.hit("Math", "Contest"));
    }
}
