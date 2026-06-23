//! `aivyx-desktop` — the native desktop shell for Aivyx.
//!
//! A thin native window that hosts the **Studio** (the Dioxus web UI the daemon
//! already serves at `http://127.0.0.1:7843`) inside a system webview, plus the
//! daemon lifecycle: it attaches to a running daemon or spawns one. This is
//! *native chrome over the existing web UI*, not a second UI — the webview runs
//! the exact same WASM Studio a browser would.
//!
//! Phase 1 (this file): window + webview + ensure-daemon. The system tray,
//! native gate notifications, global hotkey, and installer land in later phases.
//!
//! Linux note: the webview is WebKitGTK, so this needs the `webkit2gtk-4.1`
//! package at build and run time (the one real native dependency).

use std::net::TcpStream;
use std::time::{Duration, Instant};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

/// Where the daemon serves the Studio (HTTP + the `/ws` WebSocket bridge).
const STUDIO_URL: &str = "http://127.0.0.1:7843/";
const STUDIO_ADDR: &str = "127.0.0.1:7843";

/// Is the daemon's Studio reachable right now?
fn daemon_reachable() -> bool {
    let Ok(addr) = STUDIO_ADDR.parse() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Ensure a daemon is serving the Studio: attach if one is already up, otherwise
/// spawn `aivyx daemon run --web-ui` (inheriting this process's environment, so
/// `AIVYX_PASSPHRASE` etc. flow through) and wait for it to come up.
///
/// The binary is resolved from `AIVYX_BIN` if set, else `aivyx` on `PATH`. A
/// spawn failure is non-fatal — the window still opens and the Studio shows its
/// "connecting…" state, so the operator can start the daemon themselves.
fn ensure_daemon() {
    if daemon_reachable() {
        return;
    }
    let bin = std::env::var("AIVYX_BIN").unwrap_or_else(|_| "aivyx".to_string());
    match std::process::Command::new(&bin)
        .args(["daemon", "run", "--web-ui"])
        .spawn()
    {
        Ok(_child) => {
            // Poll for readiness (up to ~12s) so the first webview load lands on
            // a live Studio rather than a connection error.
            let start = Instant::now();
            while !daemon_reachable() && start.elapsed() < Duration::from_secs(12) {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        Err(e) => {
            eprintln!("aivyx-desktop: could not spawn the daemon (`{bin} daemon run --web-ui`): {e}");
            eprintln!("aivyx-desktop: start it yourself, then reopen — the window will connect.");
        }
    }
}

fn main() -> wry::Result<()> {
    ensure_daemon();

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Aivyx Studio")
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .with_min_inner_size(LogicalSize::new(720.0, 480.0))
        .build(&event_loop)
        .expect("failed to create the window");

    // The webview must outlive the event loop, so keep it bound.
    let _webview = build_webview(&window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            // Phase 1: closing the window exits. Phase 2 (tray) will hide to the
            // tray and keep the daemon running instead.
            *control_flow = ControlFlow::Exit;
        }
    });
}

/// Build the webview into the window. On Linux the webview attaches to the
/// window's GTK vbox (the documented wry+tao pattern); elsewhere it builds from
/// the raw window handle.
#[cfg(not(target_os = "linux"))]
fn build_webview(window: &tao::window::Window) -> wry::Result<wry::WebView> {
    WebViewBuilder::new().with_url(STUDIO_URL).build(window)
}

#[cfg(target_os = "linux")]
fn build_webview(window: &tao::window::Window) -> wry::Result<wry::WebView> {
    use tao::platform::unix::WindowExtUnix;
    use wry::WebViewBuilderExtUnix;
    let vbox = window
        .default_vbox()
        .expect("tao window should expose a default GTK vbox on Linux");
    WebViewBuilder::new().with_url(STUDIO_URL).build_gtk(vbox)
}
