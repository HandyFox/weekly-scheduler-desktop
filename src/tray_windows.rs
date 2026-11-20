//! Windows: an icon by the clock with Open / Open folder / Quit.

use std::path::Path;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MessageBoxW, PostThreadMessageW, TranslateMessage, MB_ICONERROR, MB_OK, MSG, WM_QUIT,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn message_box(text: &str, title: &str) {
    let (t, c) = (wide(text), wide(title));
    unsafe {
        MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

pub fn run(url: &str, data_dir: &Path) {
    let open = MenuItem::new("Open the scheduler", true, None);
    let folder = MenuItem::new("Open the folder with my schedule", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    let _ = menu.append_items(&[&open, &folder, &PredefinedMenuItem::separator(), &quit]);

    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(crate::APP_NAME);
    if let Ok(icon) = Icon::from_resource(1, Some((32, 32))) {
        builder = builder.with_icon(icon);
    }
    let tray = match builder.build() {
        Ok(t) => Some(t),
        Err(e) => {
            crate::web::log(data_dir, &format!("no tray icon: {e}"));
            None
        }
    };

    let tid = unsafe { GetCurrentThreadId() };
    let stop = move || unsafe {
        PostThreadMessageW(tid, WM_QUIT, 0, 0);
    };
    crate::web::set_quit_hook(stop);

    let (open_id, folder_id, quit_id) = (open.id().clone(), folder.id().clone(), quit.id().clone());
    let (u, d) = (url.to_string(), data_dir.to_path_buf());
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == open_id {
            crate::open_browser(&u);
        } else if e.id == folder_id {
            let _ = opener::open(&d);
        } else if e.id == quit_id {
            stop();
        }
    }));
    let u = url.to_string();
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            crate::open_browser(&u);
        }
    }));

    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    drop(tray);
}
