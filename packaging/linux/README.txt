Weekly Assignment Scheduler for Linux
=====================================

Install for just you (no sudo needed):
    ./install.sh
Then open "Weekly Assignment Scheduler" from your applications menu.

Or run it straight from this folder:
    ./weekly-scheduler

It opens in your web browser and runs entirely on this computer.
To stop it, click Quit in the top corner of the page.

Your schedule is kept in ~/Documents/Weekly Assignment Scheduler/scheduler.db,
with a backup copy in the "backups" folder there every time it starts.
Already using the scheduler? Settings -> Your data -> "Use a different scheduler.db...".

Options:
    weekly-scheduler --share        tablets on the same Wi-Fi can open it too
    weekly-scheduler --no-browser   start without opening the browser
    weekly-scheduler --port 8700    use another port (default 8642)
    weekly-scheduler --data-dir DIR keep scheduler.db somewhere else

Remove it again:
    ./install.sh --uninstall        (your schedule is kept)
