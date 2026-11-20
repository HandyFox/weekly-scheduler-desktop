# Weekly Assignment Scheduler — desktop app

Plan each week in a grid and print a sheet for every student plus a teacher copy.
This is the stand-alone version for **Windows, Mac and Linux**: one small program
(written in Rust) that opens the scheduler in your web browser and keeps everything
on your own computer. No Python, no internet, no account.

It reads and writes the same `scheduler.db` as the Python version of the scheduler,
and the printouts come out exactly the same.

## Download

Get the newest version from the **[Releases page](../../releases/latest)**:

| Computer | File | How to install |
|---|---|---|
| Windows 10 / 11 | `WeeklyScheduler-Setup-<version>.exe` | Run it. If you see "Windows protected your PC", click **More info → Run anyway**. |
| Mac (Apple Silicon or Intel, macOS 11+) | `WeeklyScheduler-<version>-mac.dmg` | Open it and drag the app to **Applications**. The first time, see [Opening it on a Mac](#opening-it-on-a-mac). |
| Ubuntu, Debian, Mint, Raspberry Pi OS | `weekly-scheduler_<version>_amd64.deb` (most PCs) or `_arm64.deb` (Raspberry Pi 4/5, ARM laptops) | Double-click it, or `sudo apt install ./weekly-scheduler_<version>_amd64.deb` |
| Other Linux | `weekly-scheduler-<version>-linux-x86_64.tar.gz` (or `aarch64`) | Unpack it and run `./install.sh` (no sudo), or just `./weekly-scheduler` |

The installers aren't signed with paid certificates yet, which is why Windows and macOS warn about them.

### Opening it on a Mac

Because the app isn't notarized by Apple yet, macOS blocks it the first time:

1. Open **Weekly Assignment Scheduler** from Applications. When macOS says it can't check the app, click **Done**.
2. Open **System Settings → Privacy & Security**, scroll down and click **Open Anyway** next to Weekly Assignment Scheduler.
3. If it asks to use your Documents folder, click **Allow** — your schedule is saved there.

You only do this once.

## Using it

- Open **Weekly Assignment Scheduler** from the Start menu, Applications folder, or your Linux app menu. Your browser opens the scheduler.
- **Windows:** a calendar icon sits by the clock (look under the ^ arrow). Click it to open the scheduler; right-click for the folder with your schedule, or Quit.
- **Mac:** a calendar icon sits in the menu bar at the top of the screen with the same choices. There is no Dock icon.
- **Everywhere:** the **Quit** button in the top corner of the page stops the scheduler. Everything is saved as you type.
- Opening the app while it is already running just opens the browser again.

### Where your schedule is kept

`Documents/Weekly Assignment Scheduler/scheduler.db`

Every time the scheduler starts, it saves a backup copy in the `backups` folder next to it (the last 15 are kept).
**Settings → Your data** can open that folder, download a copy, show the backups, and go back to one.

### Already using the scheduler?

Open **Settings → Your data → Use a different scheduler.db…** (or the button on the welcome screen) and pick your existing file.
The schedule that was there is backed up first, so this can be undone.

Uninstalling the app never deletes your schedule.

### Options

The program also takes a few options (on Windows, add them to a shortcut's target):

| Option | What it does |
|---|---|
| `--share` | Tablets and phones on the same Wi-Fi can open it too (the address is shown under Settings → Your data). Anyone on that network can then use it, so only do this on a network you trust. |
| `--no-browser` | Start without opening the browser (the Windows "start when I sign in" choice uses this). |
| `--port 8700` | Use another port. The default is 8642; if it's taken, the next free one is used. |
| `--data-dir FOLDER` | Keep `scheduler.db` in another folder. `SCHEDULER_DATA_DIR` does the same. |
| `--version` | Print the version. |

**Portable use:** put `scheduler.db` (or an empty file named `portable.txt`) next to the program, and it uses that folder instead of Documents.

## Privacy and safety

- The scheduler only answers on this computer (`127.0.0.1`) unless you start it with `--share`.
- Changes are only accepted from the scheduler's own page, so other websites open in your browser can't change your schedule.
- A file you pick with "Use a different scheduler.db" is checked before it replaces anything.

## Building it yourself

You need [Rust](https://rustup.rs).

```sh
cargo test --release
cargo build --release          # → target/release/WeeklyScheduler(.exe)
```

Packaging:

- **Windows:** install [NSIS](https://nsis.sourceforge.io), then
  `makensis -DVERSION=1.1.0 installer\installer.nsi` → `dist\WeeklyScheduler-Setup-1.1.0.exe`
- **Mac:** `rustup target add aarch64-apple-darwin x86_64-apple-darwin && scripts/package-macos.sh` → a universal `.dmg` in `dist/`
- **Linux:** `scripts/package-linux.sh` → `.deb` and `.tar.gz` in `dist/`
- From Linux you can also cross-compile the Windows installer with mingw-w64 and NSIS: `scripts/build-windows-on-linux.sh`

GitHub Actions (`.github/workflows/build.yml`) builds and tries out all of these on every push. To publish a new version, raise `version` in `Cargo.toml` (and the numbers in `assets/app.rc`) and push to `main`: when the builds pass, they appear on the Releases page under that version.

### How the code is laid out

| File | What's in it |
|---|---|
| `src/main.rs` | Start-up: data folder, port, single instance, backups, then the tray / menu bar / terminal |
| `src/web.rs` | The local web server, security checks, and the "Your data" tools |
| `src/api.rs` | The editor's API (a port of the Python app's routes) |
| `src/db.rs` | The database schema, backups and the one-time update of old files |
| `src/logic.rs` | Dates, school-day numbering, and moving lesson numbers on to the next week |
| `src/print.rs` | The printouts, rendered from the same templates as the Python app |
| `src/tray_windows.rs`, `src/tray_mac.rs` | The icon by the clock on Windows and in the menu bar on Mac |
| `assets/` | The editor page, printout templates, icons and Windows resources |
| `installer/`, `packaging/`, `scripts/` | Windows installer, Mac and Linux packaging |

Every edit in the editor was checked against the Python version on a real schedule: both make the same changes to `scheduler.db`, give the same answers, and print byte-for-byte the same pages.

To redraw the icons: `pip install pillow && python scripts/make-icon.py`.
