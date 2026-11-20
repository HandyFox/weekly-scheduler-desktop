//! The local web server: security checks, routing, and the "Your data" tools
//! (download, use a different scheduler.db, undo from a backup).

use crate::api::{self, ApiErr, Body, Res};
use crate::{db, print};
use regex::Regex;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};
use tiny_http::{Header, Request, Response};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const APP_HTML: &str = include_str!("../assets/templates/app.html");
const APP_CSS: &str = include_str!("../assets/static/app.css");
const APP_JS: &str = include_str!("../assets/static/app.js");
const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
const MAX_BODY: u64 = 60 * 1024 * 1024;

pub struct App {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub port: u16,
    pub share: bool,
    /// http://<this computer's network address>:port when --share is on
    pub share_url: Option<String>,
    /// Normal requests share it; swapping in a different scheduler.db takes it alone.
    pub lock: RwLock<()>,
}

enum Out {
    Json(u16, Value),
    Html(String),
    Text(u16, String),
    Bytes(&'static str, Vec<u8>, Option<String>),
    Static(&'static str, &'static [u8]),
    Redirect(String),
}

// ─────────────────────────────────────────────────────────────── log ──
pub fn log(data_dir: &Path, msg: &str) {
    let p = data_dir.join("scheduler.log");
    if std::fs::metadata(&p).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&p, data_dir.join("scheduler.log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{}  {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
    }
}

// ──────────────────────────────────────────────────────────── quitting ──
static QUIT_HOOK: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

/// The tray / menu-bar code sets this so Quit closes its icon properly.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
pub fn set_quit_hook(f: impl Fn() + Send + Sync + 'static) {
    let _ = QUIT_HOOK.set(Box::new(f));
}

pub fn quit() {
    match QUIT_HOOK.get() {
        Some(f) => f(),
        None => std::process::exit(0),
    }
}

// ─────────────────────────────────────────────────────────── helpers ──
fn header(r: &Request, name: &'static str) -> Option<String> {
    r.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str().to_string())
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn err(e: ApiErr) -> Out {
    Out::Json(e.0, json!({ "error": e.1 }))
}

fn json_out(r: Res) -> Out {
    match r {
        Ok(v) => Out::Json(200, v),
        Err(e) => err(e),
    }
}

impl App {
    /// Run a change inside one transaction.
    fn tx(&self, f: impl FnOnce(&Connection) -> Res) -> Out {
        let _g = self.lock.read().unwrap_or_else(|e| e.into_inner());
        let r = (|| {
            let c = db::open(&self.db_path)?;
            c.execute_batch("BEGIN IMMEDIATE")?;
            match f(&c) {
                Ok(v) => {
                    c.execute_batch("COMMIT")?;
                    Ok(v)
                }
                Err(e) => {
                    let _ = c.execute_batch("ROLLBACK");
                    Err(e)
                }
            }
        })();
        json_out(r)
    }

    fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T, ApiErr>) -> Result<T, ApiErr> {
        let _g = self.lock.read().unwrap_or_else(|e| e.into_inner());
        let c = db::open(&self.db_path)?;
        f(&c)
    }

    fn allowed_host(&self, host: &str) -> bool {
        if self.share {
            return true; // tablets use this computer's network address
        }
        let p = self.port;
        [format!("127.0.0.1:{p}"), format!("localhost:{p}"), format!("[::1]:{p}")].iter().any(|h| h.eq_ignore_ascii_case(host))
    }
}

fn app_html() -> String {
    APP_HTML
        .replace("{{ url_for('static', filename='app.css') }}?v=2", &format!("/static/app.css?v={VERSION}"))
        .replace("{{ url_for('static', filename='app.js') }}?v=2", &format!("/static/app.js?v={VERSION}"))
}

// ─────────────────────────────────────────────────────────── request ──
pub fn handle(app: &App, mut rq: Request) {
    let method = rq.method().as_str().to_ascii_uppercase();
    let url = rq.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let path = path.to_string();
    let query: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    let host = header(&rq, "Host").unwrap_or_default();

    let out = if !app.allowed_host(&host) {
        Out::Text(403, "Open the scheduler at http://127.0.0.1 on this computer.".into())
    } else if method != "GET" && method != "HEAD" && !write_allowed(&rq, &host) {
        Out::Json(403, json!({"error": "This request did not come from the scheduler page."}))
    } else if rq.body_length().is_some_and(|n| n as u64 > MAX_BODY) {
        Out::Json(413, json!({"error": "That file is too big (60 MB at most)."}))
    } else {
        let mut body = Vec::new();
        let read_ok = rq.as_reader().take(MAX_BODY + 1).read_to_end(&mut body).is_ok();
        if !read_ok {
            Out::Json(400, json!({"error": "could not read the request"}))
        } else if body.len() as u64 > MAX_BODY {
            Out::Json(413, json!({"error": "That file is too big (60 MB at most)."}))
        } else {
            route(app, &method, &path, &query, body)
        }
    };
    respond(app, rq, &method, &path, out);
}

/// Changes must come from the scheduler page itself: it adds X-Scheduler: 1,
/// which a page on another site can't do without permission we never give.
fn write_allowed(rq: &Request, host: &str) -> bool {
    if header(rq, "X-Scheduler").as_deref() != Some("1") {
        return false;
    }
    match header(rq, "Origin") {
        Some(o) => o.eq_ignore_ascii_case(&format!("http://{host}")),
        None => true,
    }
}

fn respond(app: &App, rq: Request, method: &str, path: &str, out: Out) {
    let h = |k: &str, v: &str| Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap();
    let (code, ctype, data, extra): (u16, String, Vec<u8>, Vec<Header>) = match out {
        Out::Json(code, v) => {
            if code >= 500 {
                log(&app.data_dir, &format!("{method} {path} -> {code} {v}"));
            }
            (code, "application/json".into(), serde_json::to_vec(&v).unwrap_or_default(), vec![h("Cache-Control", "no-store")])
        }
        Out::Html(s) => (200, "text/html; charset=utf-8".into(), s.into_bytes(), vec![h("Cache-Control", "no-store")]),
        Out::Text(code, s) => (code, "text/plain; charset=utf-8".into(), s.into_bytes(), vec![]),
        Out::Bytes(ct, b, name) => {
            let mut x = vec![h("Cache-Control", "no-store")];
            if let Some(n) = name {
                x.push(h("Content-Disposition", &format!("attachment; filename=\"{n}\"")));
            }
            (200, ct.into(), b, x)
        }
        Out::Static(ct, b) => (200, ct.into(), b.to_vec(), vec![h("Cache-Control", "max-age=86400")]),
        Out::Redirect(to) => (302, "text/plain".into(), Vec::new(), vec![h("Location", &to)]),
    };
    let mut r = Response::from_data(if method == "HEAD" { Vec::new() } else { data }).with_status_code(code);
    r.add_header(h("Content-Type", &ctype));
    r.add_header(h("X-Content-Type-Options", "nosniff"));
    r.add_header(h("X-Frame-Options", "SAMEORIGIN"));
    r.add_header(h("Referrer-Policy", "no-referrer"));
    for x in extra {
        r.add_header(x);
    }
    let _ = rq.respond(r);
}

fn parse_body(body: &[u8]) -> Body {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(m)) => m,
        _ => Body::new(),
    }
}

fn route(app: &App, method: &str, path: &str, query: &[(String, String)], raw: Vec<u8>) -> Out {
    let q = |k: &str| query.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
    let seg: Vec<String> = path.trim_matches('/').split('/').map(percent_decode).collect();
    let seg: Vec<&str> = seg.iter().map(|s| s.as_str()).collect();
    let b = parse_body(&raw);
    let num = |s: &str| s.parse::<i64>().ok();
    let m = if method == "HEAD" { "GET" } else { method };

    match (m, seg.as_slice()) {
        ("GET", [""]) => Out::Html(app_html()),
        ("GET", ["static", "app.css"]) => Out::Static("text/css; charset=utf-8", APP_CSS.as_bytes()),
        ("GET", ["static", "app.js"]) => Out::Static("text/javascript; charset=utf-8", APP_JS.as_bytes()),
        ("GET", ["favicon.ico"]) | ("GET", ["static", "icon.png"]) => Out::Static("image/png", ICON_PNG),
        ("GET", ["__scheduler"]) => Out::Text(200, "weekly-assignment-scheduler".into()),

        ("GET", ["api", "state"]) => {
            let dir = app.data_dir.display().to_string();
            json_out(app.read(|c| api::state(c, q("week"), &dir, app.share_url.as_deref())))
        }
        ("POST", ["api", "entries"]) => app.tx(|c| api::create_entry(c, &b)),
        ("PATCH", ["api", "entries", id]) if num(id).is_some() => app.tx(|c| api::update_entry(c, num(id).unwrap(), &b)),
        ("DELETE", ["api", "entries", id]) if num(id).is_some() => {
            let linked = q("linked") == Some("1");
            app.tx(|c| api::delete_entry(c, num(id).unwrap(), linked))
        }
        ("POST", ["api", "entries", id, act]) if num(id).is_some() => {
            let id = num(id).unwrap();
            match *act {
                "move" => app.tx(|c| api::move_entry(c, id, &b)),
                "copy" => app.tx(|c| api::copy_entry(c, id, &b)),
                "link" => app.tx(|c| api::link_entry(c, id, &b)),
                "unlink" => app.tx(|c| api::unlink_entry(c, id)),
                "fill-week" => app.tx(|c| api::fill_week(c, id)),
                _ => not_found(),
            }
        }
        ("POST", ["api", "rows"]) => app.tx(|c| api::row_action(c, &b)),
        ("POST", ["api", "week", "start"]) => app.tx(|c| api::start_week(c, &b)),
        ("POST", ["api", "week", "clear"]) => app.tx(|c| api::clear_week(c, &b)),
        ("POST", ["api", "week", "autolink"]) => app.tx(|c| api::autolink(c, &b)),
        ("POST", ["api", "weekly"]) => app.tx(|c| api::weekly_create(c, &b)),
        ("PATCH", ["api", "weekly", id]) if num(id).is_some() => app.tx(|c| api::weekly_update(c, num(id).unwrap(), &b)),
        ("DELETE", ["api", "weekly", id]) if num(id).is_some() => app.tx(|c| api::weekly_delete(c, num(id).unwrap())),
        ("PUT", ["api", "notes"]) => app.tx(|c| api::put_note(c, &b)),
        ("POST", ["api", "grades"]) => app.tx(|c| api::grade_create(c, &b)),
        ("PATCH", ["api", "grades", id]) if num(id).is_some() => app.tx(|c| api::grade_update(c, num(id).unwrap(), &b)),
        ("DELETE", ["api", "grades", id]) if num(id).is_some() => app.tx(|c| api::grade_delete(c, num(id).unwrap())),
        ("POST", ["api", "students"]) => app.tx(|c| api::student_create(c, &b)),
        ("PATCH", ["api", "students", id]) if num(id).is_some() => app.tx(|c| api::student_update(c, num(id).unwrap(), &b)),
        ("DELETE", ["api", "students", id]) if num(id).is_some() => app.tx(|c| api::student_delete(c, num(id).unwrap())),
        ("POST", ["api", "cleanup"]) => app.tx(api::cleanup),
        ("PUT", ["api", "settings"]) => app.tx(|c| api::put_settings(c, &b)),
        ("POST", ["api", "noschool"]) => app.tx(|c| api::noschool_add(c, &b)),
        ("DELETE", ["api", "noschool", day]) => app.tx(|c| api::noschool_delete(c, day)),

        ("GET", ["print", "teacher"]) => {
            let ws = crate::logic::parse_week(q("week"));
            html(app.read(|c| print::teacher(c, &ws)))
        }
        ("GET", ["print", "student", id]) if num(id).is_some() => {
            let ws = crate::logic::parse_week(q("week"));
            match app.read(|c| print::student(c, &ws, num(id).unwrap())) {
                Ok(Some(s)) => Out::Html(s),
                Ok(None) => Out::Redirect("/".into()),
                Err(e) => err(e),
            }
        }
        ("GET", ["print", "bulk"]) => {
            let ws = crate::logic::parse_week(q("week"));
            let g = q("grade").and_then(|g| g.trim().parse::<i64>().ok());
            html(app.read(|c| print::bulk(c, &ws, g)))
        }

        ("GET", ["api", "data", "download"]) => download(app),
        ("POST", ["api", "data", "upload"]) => upload(app, raw),
        ("GET", ["api", "data", "backups"]) => Out::Json(200, list_backups(app)),
        ("POST", ["api", "data", "restore"]) => restore(app, &b),
        ("POST", ["api", "data", "open-folder"]) => {
            let dir = if b.get("which").and_then(|v| v.as_str()) == Some("backups") {
                db::backup_dir(&app.db_path)
            } else {
                app.data_dir.clone()
            };
            let _ = std::fs::create_dir_all(&dir);
            match opener::open(&dir) {
                Ok(()) => Out::Json(200, json!({"ok": true})),
                Err(e) => Out::Json(500, json!({"error": format!("Could not open the folder: {e}")})),
            }
        }
        ("POST", ["api", "quit"]) => {
            log(&app.data_dir, "quit from the scheduler page");
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(400));
                quit();
            });
            Out::Json(200, json!({"ok": true}))
        }
        _ => not_found(),
    }
}

fn not_found() -> Out {
    Out::Json(404, json!({"error": "not found"}))
}

fn html(r: Result<String, ApiErr>) -> Out {
    match r {
        Ok(s) => Out::Html(s),
        Err(e) => err(e),
    }
}

// ───────────────────────────────────────────────────── your data ──
fn tmp_path(app: &App, what: &str) -> PathBuf {
    app.data_dir.join(format!(".{what}-{:08x}.tmp", fastrand::u32(..)))
}

fn download(app: &App) -> Out {
    let tmp = tmp_path(app, "download");
    let r = (|| -> Result<Vec<u8>, String> {
        let _g = app.lock.read().unwrap_or_else(|e| e.into_inner());
        let c = db::open(&app.db_path).map_err(|e| e.to_string())?;
        c.backup(rusqlite::MAIN_DB, &tmp, None).map_err(|e| e.to_string())?;
        std::fs::read(&tmp).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&tmp);
    match r {
        Ok(bytes) => {
            let name = format!("scheduler-{}.db", chrono::Local::now().format("%Y-%m-%d"));
            Out::Bytes("application/octet-stream", bytes, Some(name))
        }
        Err(e) => Out::Json(500, json!({"error": e})),
    }
}

/// A file someone picked must be a healthy scheduler database before it replaces theirs.
fn check_db(path: &Path) -> Result<(), String> {
    let mut head = [0u8; 16];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).map_err(|_| "That file is not a scheduler.db.".to_string())?;
    if &head != b"SQLite format 3\0" {
        return Err("That file is not a scheduler.db (it isn't a database file).".into());
    }
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("That file can't be opened: {e}"))?;
    let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).map_err(|e| format!("That file is damaged: {e}"))?;
    if ok != "ok" {
        return Err("That file is damaged (it failed SQLite's integrity check).".into());
    }
    let triggers: i64 = c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger'", [], |r| r.get(0)).unwrap_or(1);
    if triggers > 0 {
        return Err("That database has triggers in it, so it wasn't made by the scheduler.".into());
    }
    for t in ["grades", "students", "schedule_entries"] {
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?", [t], |r| r.get(0))
            .unwrap_or(0);
        if n == 0 {
            return Err(format!("That database has no {t} table, so it isn't a scheduler.db."));
        }
    }
    Ok(())
}

/// Replace scheduler.db with `tmp` (after backing up the current one).
fn swap_in(app: &App, tmp: &Path, tag: &str) -> Result<Value, String> {
    let _g = app.lock.write().unwrap_or_else(|e| e.into_inner());
    db::make_backup(&app.db_path, tag)?;
    let mut last = None;
    for _ in 0..20 {
        match std::fs::rename(tmp, &app.db_path) {
            Ok(()) => {
                last = None;
                break;
            }
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    if let Some(e) = last {
        return Err(format!("Could not replace scheduler.db: {e}"));
    }
    for ext in ["-journal", "-wal", "-shm"] {
        let mut p = app.db_path.clone().into_os_string();
        p.push(ext);
        let _ = std::fs::remove_file(PathBuf::from(p));
    }
    db::init_db(&app.db_path)?;
    let c = db::open(&app.db_path).map_err(|e| e.to_string())?;
    let n = |sql: &str| db::count(&c, sql, []).unwrap_or(0);
    Ok(json!({
        "ok": true,
        "grades": n("SELECT COUNT(*) FROM grades"),
        "students": n("SELECT COUNT(*) FROM students"),
        "entries": n("SELECT COUNT(*) FROM schedule_entries"),
    }))
}

fn upload(app: &App, raw: Vec<u8>) -> Out {
    let tmp = tmp_path(app, "upload");
    let r = (|| {
        std::fs::write(&tmp, &raw).map_err(|e| e.to_string())?;
        check_db(&tmp)?;
        swap_in(app, &tmp, "before-upload")
    })();
    let _ = std::fs::remove_file(&tmp);
    match r {
        Ok(v) => {
            log(&app.data_dir, "replaced scheduler.db with an uploaded file");
            Out::Json(200, v)
        }
        Err(e) => Out::Json(400, json!({"error": e})),
    }
}

static BACKUP_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^scheduler_([a-z0-9-]+)_(\d{4}-\d{2}-\d{2})_(\d{2})(\d{2})(\d{2})\.db$").unwrap());

fn list_backups(app: &App) -> Value {
    let dir = db::backup_dir(&app.db_path);
    let mut list: Vec<Value> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let m = BACKUP_NAME.captures(&name)?;
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            Some(json!({
                "name": name,
                "kind": &m[1],
                "date": &m[2],
                "time": format!("{}:{}:{}", &m[3], &m[4], &m[5]),
                "size": size,
            }))
        })
        .collect();
    list.sort_by(|a, b| {
        let k = |v: &Value| format!("{} {}", v["date"].as_str().unwrap_or(""), v["time"].as_str().unwrap_or(""));
        k(b).cmp(&k(a))
    });
    json!({ "backups": list, "folder": dir.display().to_string() })
}

fn restore(app: &App, b: &Body) -> Out {
    let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
    if !BACKUP_NAME.is_match(name) {
        return Out::Json(400, json!({"error": "unknown backup"}));
    }
    let src = db::backup_dir(&app.db_path).join(name);
    if !src.is_file() {
        return Out::Json(404, json!({"error": "That backup is gone."}));
    }
    let tmp = tmp_path(app, "restore");
    let r = (|| {
        std::fs::copy(&src, &tmp).map_err(|e| e.to_string())?;
        check_db(&tmp)?;
        swap_in(app, &tmp, "before-restore")
    })();
    let _ = std::fs::remove_file(&tmp);
    match r {
        Ok(v) => {
            log(&app.data_dir, &format!("restored {name}"));
            Out::Json(200, v)
        }
        Err(e) => Out::Json(400, json!({"error": e})),
    }
}
