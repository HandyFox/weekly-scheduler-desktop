//! macOS: a menu-bar icon with Open / Open folder / Quit, and no Dock icon.
//! Opening the app again from Finder or Launchpad just opens the browser.

use std::path::PathBuf;
use std::sync::Mutex;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};

/// 36×36 black-on-clear picture; macOS tints it to match the menu bar.
const TEMPLATE_ICON: &[u8] = include_bytes!("../assets/tray-template.rgba");

enum UserEvent {
    Menu(MenuEvent),
    Quit,
}

pub fn run(url: String, data_dir: PathBuf) -> ! {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = Mutex::new(event_loop.create_proxy());
    crate::web::set_quit_hook(move || {
        let _ = proxy.lock().map(|p| p.send_event(UserEvent::Quit));
    });
    let menu_proxy = Mutex::new(event_loop.create_proxy());
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = menu_proxy.lock().map(|p| p.send_event(UserEvent::Menu(e)));
    }));

    let open = MenuItem::new("Open the scheduler", true, None);
    let folder = MenuItem::new("Open the folder with my schedule", true, None);
    let quit = MenuItem::new("Quit Weekly Assignment Scheduler", true, None);
    let menu = Menu::new();
    let _ = menu.append_items(&[&open, &folder, &PredefinedMenuItem::separator(), &quit]);
    let mut menu = Some(menu);
    let mut tray = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                let mut b = TrayIconBuilder::new().with_tooltip(crate::APP_NAME).with_icon_as_template(true);
                if let Some(m) = menu.take() {
                    b = b.with_menu(Box::new(m));
                }
                if let Ok(icon) = Icon::from_rgba(TEMPLATE_ICON.to_vec(), 36, 36) {
                    b = b.with_icon(icon);
                }
                match b.build() {
                    Ok(t) => tray = Some(t),
                    Err(e) => crate::web::log(&data_dir, &format!("no menu-bar icon: {e}")),
                }
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                if e.id == *open.id() {
                    crate::open_browser(&url);
                } else if e.id == *folder.id() {
                    let _ = opener::open(&data_dir);
                } else if e.id == *quit.id() {
                    tray.take();
                    std::process::exit(0);
                }
            }
            Event::UserEvent(UserEvent::Quit) => {
                tray.take();
                std::process::exit(0);
            }
            // double-clicked in Finder while already running
            Event::Reopen { .. } => crate::open_browser(&url),
            _ => {}
        }
    })
}
