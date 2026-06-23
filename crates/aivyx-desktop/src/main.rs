//! `aivyx-desktop` — the native desktop shell for Aivyx.
//!
//! A thin native window that hosts the **Studio** (the Dioxus web UI the daemon
//! already serves at `http://127.0.0.1:7843`) inside a system webview, plus the
//! daemon lifecycle and a system tray. This is *native chrome over the existing
//! web UI*, not a second UI — the webview runs the exact same WASM Studio a
//! browser would.
//!
//! Phases: (1) window + webview + ensure-daemon, (2 — this file) **system tray**
//! (Open Studio / Restart daemon / Quit) with hide-to-tray on close. Native gate
//! notifications, a global hotkey, and the installer land in later phases.
//!
//! Linux note: the webview is WebKitGTK (`webkit2gtk-4.1`) and the tray uses an
//! appindicator (`libayatana-appindicator`) — the shell's native dependencies.

use std::net::TcpStream;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Window, WindowBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIconBuilder, TrayIconEvent};
use wry::WebViewBuilder;

/// Where the daemon serves the Studio (HTTP + the `/ws` WebSocket bridge).
const STUDIO_URL: &str = "http://127.0.0.1:7843/";
const STUDIO_ADDR: &str = "127.0.0.1:7843";

/// Events we route into the single tao event loop from the tray's global
/// channels, so everything is handled in one place.
enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
}

/// The `aivyx` binary to drive the daemon: `AIVYX_BIN` if set, else `aivyx` on
/// `PATH`.
fn aivyx_bin() -> String {
    std::env::var("AIVYX_BIN").unwrap_or_else(|_| "aivyx".to_string())
}

/// Is the daemon's Studio reachable right now?
fn daemon_reachable() -> bool {
    let Ok(addr) = STUDIO_ADDR.parse() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Block (up to ~12s) until the daemon is serving, so the first webview load
/// lands on a live Studio.
fn wait_until_reachable() {
    let start = Instant::now();
    while !daemon_reachable() && start.elapsed() < Duration::from_secs(12) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Ensure a daemon is serving the Studio: attach if one is up (returns `None`),
/// else spawn `aivyx daemon run --web-ui` (env inherited) and return the child
/// so we can stop it on quit. A spawn failure is non-fatal.
fn ensure_daemon() -> Option<Child> {
    if daemon_reachable() {
        return None;
    }
    match Command::new(aivyx_bin())
        .args(["daemon", "run", "--web-ui"])
        .spawn()
    {
        Ok(child) => {
            wait_until_reachable();
            Some(child)
        }
        Err(e) => {
            eprintln!("aivyx-desktop: could not spawn the daemon: {e}");
            eprintln!("aivyx-desktop: start it yourself, then reopen — the window will connect.");
            None
        }
    }
}

/// Stop a daemon we own (graceful `aivyx daemon stop`, then reap the child).
fn stop_owned_daemon(child: &mut Option<Child>) {
    if child.is_none() {
        return;
    }
    let _ = Command::new(aivyx_bin()).args(["daemon", "stop"]).status();
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Decode the embedded brand PNG into a tray icon.
fn tray_icon_image() -> tray_icon::Icon {
    let bytes = include_bytes!("../assets/tray-icon.png");
    let decoder = png::Decoder::new(&bytes[..]);
    let mut reader = decoder.read_info().expect("valid tray PNG");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("decode tray PNG");
    let rgba = buf[..info.buffer_size()].to_vec();
    tray_icon::Icon::from_rgba(rgba, info.width, info.height).expect("RGBA -> tray Icon")
}

fn main() -> wry::Result<()> {
    let mut daemon_child = ensure_daemon();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // Route the tray's global menu/icon events into this event loop.
    let proxy = event_loop.create_proxy();
    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = menu_proxy.send_event(UserEvent::Menu(e));
    }));
    TrayIconEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(UserEvent::Tray(e));
    }));

    let window = WindowBuilder::new()
        .with_title("Aivyx Studio")
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .with_min_inner_size(LogicalSize::new(720.0, 480.0))
        .build(&event_loop)
        .expect("failed to create the window");

    let webview = build_webview(&window)?;

    // Tray menu: Open Studio · Restart daemon · Quit. Built after the event loop
    // (GTK is initialized by then on Linux).
    let menu = Menu::new();
    let open_item = MenuItem::new("Open Studio", true, None);
    let restart_item = MenuItem::new("Restart daemon", true, None);
    let quit_item = MenuItem::new("Quit Aivyx", true, None);
    menu.append_items(&[
        &open_item,
        &PredefinedMenuItem::separator(),
        &restart_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ])
    .expect("build tray menu");
    let _tray = TrayIconBuilder::new()
        .with_tooltip("Aivyx")
        .with_icon(tray_icon_image())
        .with_menu(Box::new(menu))
        .build()
        .expect("build tray icon");

    let open_id = open_item.id().clone();
    let restart_id = restart_item.id().clone();
    let quit_id = quit_item.id().clone();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            // Closing the window hides to the tray and keeps the daemon running
            // (the always-on-assistant model) — "Quit Aivyx" is the real exit.
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                window.set_visible(false);
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                if e.id == open_id {
                    window.set_visible(true);
                    window.set_focus();
                } else if e.id == restart_id {
                    stop_owned_daemon(&mut daemon_child);
                    daemon_child = ensure_daemon();
                    let _ = webview.load_url(STUDIO_URL);
                } else if e.id == quit_id {
                    stop_owned_daemon(&mut daemon_child);
                    *control_flow = ControlFlow::Exit;
                }
            }
            // Left-click on the tray icon shows/focuses the window.
            Event::UserEvent(UserEvent::Tray(TrayIconEvent::Click { .. })) => {
                window.set_visible(true);
                window.set_focus();
            }
            _ => {}
        }
    });
}

/// Build the webview into the window. On Linux the webview attaches to the
/// window's GTK vbox (the documented wry+tao pattern); elsewhere it builds from
/// the raw window handle.
#[cfg(not(target_os = "linux"))]
fn build_webview(window: &Window) -> wry::Result<wry::WebView> {
    WebViewBuilder::new().with_url(STUDIO_URL).build(window)
}

#[cfg(target_os = "linux")]
fn build_webview(window: &Window) -> wry::Result<wry::WebView> {
    use tao::platform::unix::WindowExtUnix;
    use wry::WebViewBuilderExtUnix;
    let vbox = window
        .default_vbox()
        .expect("tao window should expose a default GTK vbox on Linux");
    WebViewBuilder::new().with_url(STUDIO_URL).build_gtk(vbox)
}
