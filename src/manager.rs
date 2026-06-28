// src/manager.rs

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box, Button, Orientation, ScrolledWindow, 
    TextView, Label, PolicyType, Align, Image, Justification, HeaderBar
};
use gtk::gdk;
use std::process::Command;
use std::rc::Rc;
use std::cell::RefCell;

fn get_logs() -> String {
    // 1. Attempt to pull logs gracefully from the systemd journal framework first
    if let Ok(output) = Command::new("journalctl")
        .args(&["--user", "-u", "lens-for-gnome.service", "-n", "1000", "--no-pager"])
        .output() 
    {
        let logs = String::from_utf8_lossy(&output.stdout).to_string();
        if !logs.trim().is_empty() && !logs.contains("No entries") {
            return logs;
        }
    }
    
    // 2. Fallback to the raw manual daemon log for developer execution modes
    let home = std::env::var("HOME").unwrap_or_default();
    let log_path = format!("{}/.local/state/lens-for-gnome/daemon.log", home);
    if let Ok(content) = std::fs::read_to_string(&log_path) {
        return content;
    }
    
    "No logs found. Start the engine to generate initialization logs.".to_string()
}

fn check_status() -> (bool, bool) {
    // Check if the service is currently running
    let is_active = if let Ok(output) = Command::new("systemctl")
        .args(&["--user", "is-active", "lens-for-gnome.service"])
        .output() 
    {
        String::from_utf8_lossy(&output.stdout).trim() == "active"
    } else {
        false
    };

    // Check if the service is enabled to launch automatically at login/boot
    let is_enabled = if let Ok(output) = Command::new("systemctl")
        .args(&["--user", "is-enabled", "lens-for-gnome.service"])
        .output() 
    {
        String::from_utf8_lossy(&output.stdout).trim() == "enabled"
    } else {
        false
    };

    (is_active, is_enabled)
}

fn start_daemon() {
    let _ = Command::new("systemctl").args(&["--user", "start", "lens-for-gnome.service"]).spawn();
}

fn stop_daemon() {
    let _ = Command::new("systemctl").args(&["--user", "stop", "lens-for-gnome.service"]).spawn();
}

fn restart_daemon() {
    let _ = Command::new("systemctl").args(&["--user", "restart", "lens-for-gnome.service"]).spawn();
}

fn toggle_autostart(enable: bool) {
    if enable {
        let _ = Command::new("systemctl").args(&["--user", "enable", "lens-for-gnome.service"]).spawn();
    } else {
        let _ = Command::new("systemctl").args(&["--user", "disable", "lens-for-gnome.service"]).spawn();
    }
}

fn update_icon_button(btn: &Button, icon_name: &str, label_text: &str) {
    let box_ = Box::new(Orientation::Horizontal, 6);
    box_.set_halign(Align::Center);
    let icon = Image::from_icon_name(icon_name);
    let label = Label::new(Some(label_text));
    box_.append(&icon);
    box_.append(&label);
    btn.set_child(Some(&box_));
}

fn ensure_desktop_integration() {
    let home = std::env::var("HOME").unwrap_or_default();
    let app_dir = format!("{}/.local/share/applications", home);
    let desktop_path = format!("{}/org.gnome.Lens.desktop", app_dir);
    
    let Ok(current_exe) = std::env::current_exe() else { return };
    let Ok(current_dir) = std::env::current_dir() else { return };
    
    let icon_path = current_dir.join("metadata").join("io.github.cwittenberg.Lens.icon.svg");
    
    if !std::path::Path::new(&app_dir).exists() {
        let _ = std::fs::create_dir_all(&app_dir);
    }

    // Creating this desktop file dynamically forces GNOME Shell (Wayland) to bind the Application ID 
    // to the absolute path of the local SVG file, enabling native dock integration during development.
    let desktop_content = format!(
        "[Desktop Entry]\n\
        Version=1.0\n\
        Type=Application\n\
        Name=Lens for GNOME\n\
        Exec={}\n\
        Icon={}\n\
        Terminal=false\n\
        StartupNotify=true\n",
        current_exe.to_string_lossy(),
        icon_path.to_string_lossy()
    );

    if std::fs::write(&desktop_path, desktop_content).is_ok() {
        // Trigger GNOME to aggressively refresh its internal application caches
        let _ = Command::new("update-desktop-database").arg(&app_dir).spawn();
    }
}

fn build_ui(app: &Application) {
    // Manually register the metadata directory so GTK can find the local SVG icon if running from source
    if let Some(display) = gdk::Display::default() {
        let icon_theme = gtk::IconTheme::for_display(&display);
        if let Ok(cur_dir) = std::env::current_dir() {
            let metadata_dir = cur_dir.join("metadata");
            if metadata_dir.exists() {
                icon_theme.add_search_path(&metadata_dir);
            }
        }
    }

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Lens for GNOME")
        .icon_name("org.gnome.Lens")
        .default_width(1050)
        .default_height(650)
        .build();

    let header_bar = HeaderBar::new();
    header_bar.set_show_title_buttons(true);
    
    header_bar.set_decoration_layout(Some("icon:minimize,maximize,close"));
    window.set_titlebar(Some(&header_bar));

    let main_box = Box::new(Orientation::Vertical, 10);
    main_box.set_margin_top(15);
    main_box.set_margin_bottom(15);
    main_box.set_margin_start(15);
    main_box.set_margin_end(15);

    let header_label = Label::new(None);
    header_label.set_markup("\n<small>Use this management utility to monitor logs, manage the backend systemd service, and control the search indexer.</small>");
    header_label.set_justify(Justification::Left);
    header_label.set_margin_bottom(10);

    let controls_box = Box::new(Orientation::Horizontal, 10);
    controls_box.set_halign(Align::Center);

    let status_label = Label::new(Some("Status: Checking Engine State..."));
    status_label.set_margin_end(20);
    
    let start_btn = Button::new();
    let stop_btn = Button::new();
    let restart_btn = Button::new();
    let autostart_btn = Button::new();
    let copy_btn = Button::new();

    update_icon_button(&start_btn, "media-playback-start-symbolic", "Start Engine");
    update_icon_button(&stop_btn, "media-playback-stop-symbolic", "Stop Engine");
    update_icon_button(&restart_btn, "view-refresh-symbolic", "Restart");
    update_icon_button(&autostart_btn, "system-run-symbolic", "Checking Autostart...");
    update_icon_button(&copy_btn, "edit-copy-symbolic", "Copy Logs");

    controls_box.append(&status_label);
    controls_box.append(&start_btn);
    controls_box.append(&stop_btn);
    controls_box.append(&restart_btn);
    controls_box.append(&autostart_btn);
    controls_box.append(&copy_btn);

    let text_view = TextView::new();
    text_view.set_editable(false);
    text_view.set_cursor_visible(false);
    text_view.set_monospace(true);
    text_view.set_vexpand(true);
    
    text_view.set_left_margin(10);
    text_view.set_right_margin(10);
    text_view.set_top_margin(10);
    text_view.set_bottom_margin(10);

    let text_buffer = text_view.buffer();
    
    let scrolled_window = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .child(&text_view)
        .vexpand(true)
        .build();

    main_box.append(&header_label);
    main_box.append(&controls_box);
    main_box.append(&scrolled_window);
    window.set_child(Some(&main_box));

    let is_enabled_state = Rc::new(RefCell::new(false));

    // Handle instant UI feedback on click events before the next poll cycle occurs
    let start_btn_clone = start_btn.clone();
    let sl_clone = status_label.clone();
    start_btn.connect_clicked(move |_| {
        start_btn_clone.set_sensitive(false);
        sl_clone.set_markup("<b>Status: <span foreground='orange'>Starting...</span></b>");
        start_daemon();
    });

    let stop_btn_clone = stop_btn.clone();
    let sl_clone2 = status_label.clone();
    stop_btn.connect_clicked(move |_| {
        stop_btn_clone.set_sensitive(false);
        sl_clone2.set_markup("<b>Status: <span foreground='orange'>Stopping...</span></b>");
        stop_daemon();
    });

    let restart_btn_clone = restart_btn.clone();
    let sl_clone3 = status_label.clone();
    restart_btn.connect_clicked(move |_| {
        restart_btn_clone.set_sensitive(false);
        sl_clone3.set_markup("<b>Status: <span foreground='orange'>Restarting...</span></b>");
        restart_daemon();
    });

    let is_enabled_clone = is_enabled_state.clone();
    let autostart_btn_clone = autostart_btn.clone();
    autostart_btn.connect_clicked(move |_| {
        autostart_btn_clone.set_sensitive(false);
        let current = *is_enabled_clone.borrow();
        toggle_autostart(!current);
    });

    let last_logs_len = Rc::new(RefCell::new(0));

    let text_buffer_weak = text_buffer.downgrade();
    let window_weak = window.downgrade();

    copy_btn.connect_clicked(move |_| {
        let text_buffer = match text_buffer_weak.upgrade() {
            Some(b) => b,
            None => return,
        };
        let window = match window_weak.upgrade() {
            Some(w) => w,
            None => return,
        };
        let (start, end) = text_buffer.bounds();
        let text = text_buffer.text(&start, &end, false);
        
        let clipboard = WidgetExt::display(&window).clipboard();
        clipboard.set_text(&text);
    });

    let tb_weak = text_buffer.downgrade();
    let sl_weak = status_label.downgrade();
    let tv_weak = text_view.downgrade();
    
    let start_weak = start_btn.downgrade();
    let stop_weak = stop_btn.downgrade();
    let restart_weak = restart_btn.downgrade();
    let as_weak = autostart_btn.downgrade();

    // Poll the engine status asynchronously and update the UI
    glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
        let text_buffer = match tb_weak.upgrade() {
            Some(b) => b,
            None => return glib::ControlFlow::Break,
        };
        let status_label = match sl_weak.upgrade() {
            Some(l) => l,
            None => return glib::ControlFlow::Break,
        };
        let start_btn = match start_weak.upgrade() {
            Some(b) => b,
            None => return glib::ControlFlow::Break,
        };
        let stop_btn = match stop_weak.upgrade() {
            Some(b) => b,
            None => return glib::ControlFlow::Break,
        };
        let restart_btn = match restart_weak.upgrade() {
            Some(b) => b,
            None => return glib::ControlFlow::Break,
        };
        let autostart_btn = match as_weak.upgrade() {
            Some(b) => b,
            None => return glib::ControlFlow::Break,
        };

        let (is_running, is_enabled) = check_status();

        if is_running {
            status_label.set_markup("<b>Status: <span foreground='green'>Running</span></b>");
            start_btn.set_sensitive(false);
            stop_btn.set_sensitive(true);
            restart_btn.set_sensitive(true);
        } else {
            status_label.set_markup("<b>Status: <span foreground='red'>Stopped</span></b>");
            start_btn.set_sensitive(true);
            stop_btn.set_sensitive(false);
            restart_btn.set_sensitive(false);
        }

        autostart_btn.set_sensitive(true);
        if is_enabled != *is_enabled_state.borrow() {
            *is_enabled_state.borrow_mut() = is_enabled;
        }

        if is_enabled {
            update_icon_button(&autostart_btn, "system-lock-screen-symbolic", "Disable Autostart");
        } else {
            update_icon_button(&autostart_btn, "system-run-symbolic", "Enable Autostart");
        }

        let logs = get_logs();
        
        if logs.len() != *last_logs_len.borrow() {
            text_buffer.set_text(&logs);
            *last_logs_len.borrow_mut() = logs.len();
            
            // Defers the scroll logic until GTK has time to process the geometry change
            let tv_weak2 = tv_weak.clone();
            let tb_weak2 = tb_weak.clone();
            glib::idle_add_local(move || {
                if let (Some(tv), Some(tb)) = (tv_weak2.upgrade(), tb_weak2.upgrade()) {
                    let mut iter = tb.end_iter();
                    tv.scroll_to_iter(&mut iter, 0.0, true, 0.0, 1.0);
                }
                glib::ControlFlow::Break
            });
        }
        
        glib::ControlFlow::Continue
    });

    window.present();
}

fn main() -> glib::ExitCode {
    ensure_desktop_integration();

    let app = Application::builder()
        .application_id("org.gnome.Lens")
        .build();

    // Hook global GTK initialization calls (like default icons) to the startup signal
    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name("org.gnome.Lens");
    });

    app.connect_activate(build_ui);
    app.run()
}

