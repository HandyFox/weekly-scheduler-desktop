Weekly Assignment Scheduler for Mac
===================================

Install
  Drag "Weekly Assignment Scheduler" onto the Applications folder.

First time you open it
  The app isn't notarized by Apple yet, so macOS stops it the first time:
  1. Open it from Applications. When macOS says it can't check the app, click Done.
  2. Open System Settings -> Privacy & Security, scroll down, and click
     "Open Anyway" next to Weekly Assignment Scheduler. Confirm with your password.
  You only do this once. (Or in Terminal:
     xattr -dr com.apple.quarantine "/Applications/Weekly Assignment Scheduler.app")
  If it asks to use your Documents folder, click Allow: your schedule is saved there.

Using it
  It opens in your web browser and runs entirely on this Mac. A small calendar
  icon sits in the menu bar at the top of the screen: click it to open the
  scheduler, open the folder with your schedule, or Quit.
  The Quit button in the top corner of the page works too.

Where your schedule is kept
  Documents/Weekly Assignment Scheduler/scheduler.db, with a backup copy in the
  "backups" folder there every time it starts.
  Already using the scheduler? Settings -> Your data -> "Use a different scheduler.db...".
