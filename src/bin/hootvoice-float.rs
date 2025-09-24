// Linux + wayland_layer only: gate imports and implementation
#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
use gls::{Edge, KeyboardMode, Layer, LayerShell};
#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
use gtk::prelude::*;
#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
use gtk4 as gtk;
#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
use gtk4_layer_shell as gls;

#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
use hootvoice::utils::app_config_dir;
#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
use std::time::Duration;

#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
fn settings_path() -> std::path::PathBuf {
    app_config_dir().join("settings.toml")
}

#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
fn load_floating_position() -> Option<(i32, i32)> {
    let path = settings_path();
    let txt = std::fs::read_to_string(&path).ok()?;
    let val: toml::Value = toml::from_str(&txt).ok()?;
    if let Some(arr) = val.get("floating_position").and_then(|v| v.as_array()) {
        if arr.len() == 2 {
            let x = arr[0].as_float().unwrap_or(0.0) as i32;
            let y = arr[1].as_float().unwrap_or(0.0) as i32;
            return Some((x.max(0), y.max(0)));
        }
    }
    None
}

#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
fn save_floating_position(x: i32, y: i32) {
    let path = settings_path();
    let mut root: toml::Value = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_else(|| toml::Value::Table(Default::default()))
    } else {
        toml::Value::Table(Default::default())
    };
    // set floating_position = [x, y]
    let tbl = root.as_table_mut().unwrap();
    tbl.insert(
        "floating_position".to_string(),
        toml::Value::Array(vec![
            toml::Value::Float(x as f64),
            toml::Value::Float(y as f64),
        ]),
    );
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, toml::to_string(&root).unwrap_or_default());
}

#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
fn status_path() -> std::path::PathBuf {
    app_config_dir().join("status.json")
}

#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
fn send_signal(sig: i32) {
    if let Ok(pid_str) = std::env::var("HOOTVOICE_PARENT_PID") {
        if let Ok(pid) = pid_str.parse::<i32>() {
            unsafe {
                libc::kill(pid, sig);
            }
        }
    }
}

#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
fn main() {
    // Only useful on Wayland; if not supported, just exit.
    if !gls::is_supported() {
        return;
    }

    let app = gtk::Application::builder()
        .application_id("com.hootvoice.FloatingLayer")
        .build();

    app.connect_activate(|app| {
        let win = gtk::Window::builder()
            .application(app)
            .decorated(false)
            .resizable(false)
            .default_width(120)
            .default_height(28)
            .build();

        // Layer shell setup (overlay, free position via top/left margins)
        win.init_layer_shell();
        win.set_layer(Layer::Overlay);
        win.set_anchor(Edge::Top, true);
        win.set_anchor(Edge::Left, true);
        // Restore last position (else use 120,120)
        if let Some((x, y)) = load_floating_position() {
            win.set_margin(Edge::Left, x);
            win.set_margin(Edge::Top, y);
        } else {
            win.set_margin(Edge::Top, 120);
            win.set_margin(Edge::Left, 120);
        }
        win.set_exclusive_zone(0);
        win.set_keyboard_mode(KeyboardMode::OnDemand);

        // Global CSS (rounded, semi-transparent background)
        let provider = gtk::CssProvider::new();
        let css = r#"
            .hv-float {
                border-radius: 6px;
                background-color: rgba(24,24,24,0.86);
                border: 1px solid rgba(200,200,200,0.16);
            }
            .hv-state-red { color: rgb(220,53,69); }
            .hv-state-green { color: rgb(40,167,69); }
            .hv-state-yellow { color: rgb(255,193,7); }
        "#;
        provider.load_from_data(css);
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        // Content
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        root.add_css_class("hv-float");
        root.set_margin_top(2);
        root.set_margin_bottom(2);
        root.set_margin_start(6);
        root.set_margin_end(4);

        // Minimal controls: drag-handle, toggle, settings
        let lbl_handle = gtk::Label::new(Some("⠿"));
        let btn_toggle = gtk::Button::with_label("⏺");
        let btn_settings = gtk::Button::with_label("⚙");

        lbl_handle.set_margin_end(4);
        btn_toggle.set_margin_end(4);

        root.append(&lbl_handle);
        root.append(&btn_toggle);
        root.append(&btn_settings);
        win.set_child(Some(&root));

        // Poll status.json to update icon/state
        let btn_toggle_c = btn_toggle.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(200), move || {
            let path = status_path();
            if let Ok(txt) = std::fs::read_to_string(&path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                    let alt = v.get("alt").and_then(|x| x.as_str()).unwrap_or("");
                    // Remove state color classes first
                    btn_toggle_c.remove_css_class("hv-state-red");
                    btn_toggle_c.remove_css_class("hv-state-green");
                    btn_toggle_c.remove_css_class("hv-state-yellow");
                    match alt {
                        "rec" => {
                            btn_toggle_c.set_label("■");
                            btn_toggle_c.add_css_class("hv-state-red");
                        }
                        "proc" => {
                            btn_toggle_c.set_label("…");
                            btn_toggle_c.add_css_class("hv-state-yellow");
                        }
                        _ => {
                            btn_toggle_c.set_label("⏺");
                            btn_toggle_c.add_css_class("hv-state-green");
                        }
                    }
                }
            }
            gtk::glib::ControlFlow::Continue
        });

        // Drag to move: track top/left margins
        let drag = gtk::GestureDrag::new();
        root.add_controller(drag.clone());
        let win_c = win.clone();
        let start_pos = std::rc::Rc::new(std::cell::Cell::new((120i32, 120i32)));
        let start_pos_c = start_pos.clone();
        drag.connect_drag_begin(move |_, _x, _y| {
            // Remember current margins
            let top = win_c.margin(Edge::Top);
            let left = win_c.margin(Edge::Left);
            start_pos_c.set((left, top));
        });
        let win_c = win.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let (base_left, base_top) = start_pos.get();
            let new_left = base_left.saturating_add(dx as i32);
            let new_top = base_top.saturating_add(dy as i32);
            win_c.set_margin(Edge::Left, new_left.max(0));
            win_c.set_margin(Edge::Top, new_top.max(0));
        });

        // Save position when drag ends
        let win_c = win.clone();
        drag.connect_drag_end(move |_, _x, _y| {
            let left = win_c.margin(Edge::Left);
            let top = win_c.margin(Edge::Top);
            save_floating_position(left.max(0), top.max(0));
        });

        // Note: centering is compositor-dependent; keep default margins.

        // Buttons
        btn_toggle.connect_clicked(move |_| {
            // Toggle recording in parent
            send_signal(libc::SIGUSR1);
        });

        let win_c = win.clone();
        btn_settings.connect_clicked(move |_| {
            // Ask parent to show settings, then exit this sidecar
            send_signal(libc::SIGUSR2);
            win_c.hide();
            // Graceful exit
            std::process::exit(0);
        });

        win.show();
    });

    app.run();
}

// Fallback stub for non-Linux or when feature is disabled
#[cfg(not(all(target_os = "linux", feature = "wayland_layer")))]
fn main() {
    eprintln!("[float] Floating window is supported only on Linux with 'wayland_layer' feature");
}
