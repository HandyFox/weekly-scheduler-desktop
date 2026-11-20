// No console window on Windows; the tray icon is the app's face there.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod api;
mod db;
mod logic;
mod print;
#[cfg(target_os = "macos")]
mod tray_mac;
#[cfg(windows)]
mod tray_windows;
mod web;

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub const APP_NAME: &str = "Weekly Assignment Scheduler";
const DEFAULT_PORT: u16 = 8642; // 5000 is taken by AirPlay on newer Macs

struct Args {
    share: bool,
    no_browser: bool,
    port: Option<u16>,
    data_dir: Option<PathBuf>,
}

const HELP: &str = "Weekly Assignment Scheduler

Usage: WeeklyScheduler [options]
  --share            let tablets and phones on the same Wi-Fi open it too
  --no-browser       start without opening the browser
  --port N           first port to try (default 8642)
  --data-dir FOLDER  keep scheduler.db in FOLDER
  --version          print the version
";

fn parse_args() -> Args {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut a = Args {
        share: env("SCHEDULER_SHARE").as_deref() == Some("1"),
        no_browser: env("SCHEDULER_NO_BROWSER").as_deref() == Some("1"),
        port: env("SCHEDULER_PORT").and_then(|p| p.parse().ok()),
        data_dir: env("SCHEDULER_DATA_DIR").map(PathBuf::from),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let (k, inline) = match arg.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (arg.clone(), None),
        };
        match k.as_str() {
            "--share" => a.share = true,
            "--no-browser" => a.no_browser = true,
            "--port" => a.port = inline.or_else(|| it.next()).and_then(|p| p.parse().ok()),
            "--data-dir" => a.data_dir = inline.or_else(|| it.next()).map(PathBuf::from),
            "--version" | "-V" => {
                println!("{APP_NAME} {}", web::VERSION);
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            _ => {} // e.g. -psn_0_1234 that old macOS adds; ignore
        }
    }
    a
}

/// Where scheduler.db lives: --data-dir, a portable copy next to the program,
/// or Documents/Weekly Assignment Scheduler.
fn data_dir(args: &Args) -> PathBuf {
    if let Some(d) = &args.data_dir {
        return d.clone();
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        if dir.join("scheduler.db").is_file() || dir.join("portable.txt").is_file() {
            return dir;
        }
    }
    let docs = dirs::document_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Documents")).filter(|d| d.is_dir()))
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    docs.join(APP_NAME)
}

/// Is our own scheduler already answering on this port?
fn already_running(port: u16) -> bool {
    use std::io::{Read, Write};
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(700)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(1500)));
    let _ = s.set_write_timeout(Some(Duration::from_millis(1500)));
    if s.write_all(format!("GET /__scheduler HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()).is_err() {
        return false;
    }
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    buf.trim_end().ends_with("weekly-assignment-scheduler")
}

fn port_free(host: [u8; 4], port: u16) -> bool {
    TcpListener::bind(SocketAddr::from((host, port))).is_ok()
}

fn lan_address() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("10.255.255.255:1").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

pub fn open_browser(url: &str) {
    if opener::open_browser(url).is_err() {
        let _ = opener::open(url);
    }
}

/// Tell the person what went wrong, even when there is no terminal window.
fn fatal(msg: &str) -> ! {
    eprintln!("{APP_NAME}: {msg}");
    #[cfg(windows)]
    tray_windows::message_box(msg, APP_NAME);
    #[cfg(target_os = "macos")]
    {
        let esc = msg.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = std::process::Command::new("osascript")
            .args(["-e", &format!("display alert \"{APP_NAME}\" message \"{esc}\" as critical")])
            .status();
    }
    std::process::exit(1);
}

fn main() {
    let args = parse_args();
    let dir = data_dir(&args);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        fatal(&format!("Could not create the folder for your schedule ({}): {e}", dir.display()));
    }
    let db_path = dir.join("scheduler.db");
    let host: [u8; 4] = if args.share { [0, 0, 0, 0] } else { [127, 0, 0, 1] };
    let first = args.port.unwrap_or(DEFAULT_PORT);

    let mut port = None;
    for p in first..first.saturating_add(10) {
        if already_running(p) {
            // started twice: just show the one that's running
            println!("The scheduler is already running at http://127.0.0.1:{p}");
            if !args.no_browser {
                open_browser(&format!("http://127.0.0.1:{p}"));
            }
            return;
        }
        if port_free(host, p) {
            port = Some(p);
            break;
        }
    }
    let Some(port) = port else {
        fatal(&format!("Ports {first}-{} are all in use. Close other programs or start with --port.", first + 9));
    };

    if let Err(e) = db::make_backup(&db_path, "startup") {
        web::log(&dir, &format!("startup backup failed: {e}"));
    }
    if let Err(e) = db::init_db(&db_path) {
        fatal(&format!("Could not open your schedule ({}): {e}", db_path.display()));
    }
    let server = match tiny_http::Server::http(SocketAddr::from((host, port))) {
        Ok(s) => Arc::new(s),
        Err(e) => fatal(&format!("Could not start on port {port}: {e}")),
    };
    let lan = if args.share { lan_address().map(|ip| format!("http://{ip}:{port}")) } else { None };
    let app = Arc::new(web::App {
        data_dir: dir.clone(),
        db_path: db_path.clone(),
        port,
        share: args.share,
        share_url: lan.clone(),
        lock: RwLock::new(()),
    });
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let (server, app) = (server.clone(), app.clone());
            std::thread::spawn(move || {
                while let Ok(rq) = server.recv() {
                    web::handle(&app, rq);
                }
            })
        })
        .collect();

    let url = format!("http://127.0.0.1:{port}");
    web::log(&dir, &format!("started {} on {url}{}", web::VERSION, lan.as_ref().map(|l| format!(" and {l}")).unwrap_or_default()));
    if !args.no_browser {
        open_browser(&url);
    }

    #[cfg(windows)]
    {
        let _ = workers;
        tray_windows::run(&url, &dir);
        std::process::exit(0);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = workers;
        tray_mac::run(url, dir);
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        println!();
        println!("  {APP_NAME} is running:  {url}");
        if args.share {
            if let Some(lan) = &lan {
                println!("  On a tablet or phone on the same Wi-Fi:  {lan}");
            }
            println!("  (Anyone on this network can open it. Leave off --share to keep it to this computer.)");
        }
        println!("  Your schedule is saved in: {}", db_path.display());
        println!("  To stop it, click Quit in the scheduler page (or press Ctrl+C here).");
        println!();
        for w in workers {
            let _ = w.join();
        }
    }
}
