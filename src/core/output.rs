use std::sync::{Arc, Mutex};

#[cfg(target_os = "macos")]
mod macos_helpers {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::CStr;

    pub fn hide_app() {
        use cocoa::appkit::NSApp;
        unsafe {
            let app = NSApp();
            let _: () = msg_send![app, hide: nil];
        }
    }

    pub fn set_clipboard(text: &str) -> bool {
        unsafe {
            let pb: id = msg_send![class!(NSPasteboard), generalPasteboard];
            let _: () = msg_send![pb, clearContents];
            let s = NSString::alloc(nil).init_str(text);
            let arr: id = msg_send![class!(NSArray), arrayWithObject: s];
            let ok: i64 = msg_send![pb, writeObjects: arr];
            ok != 0
        }
    }

    pub fn frontmost_bundle_id() -> Option<String> {
        unsafe {
            let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
            let app: id = msg_send![ws, frontmostApplication];
            if app == nil {
                return None;
            }
            let bid: id = msg_send![app, bundleIdentifier];
            if bid == nil {
                return None;
            }
            let ptr: *const std::os::raw::c_char = msg_send![bid, UTF8String];
            if ptr.is_null() {
                return None;
            }
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        }
    }

    pub fn activate_bundle_id(bundle_id: &str) -> bool {
        let script = format!(
            "try\n tell application id \"{}\" to activate\nend try",
            bundle_id
        );
        std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

use crate::core::LogCallback;

#[derive(Clone, Copy, Debug)]
pub struct BehaviorOptions {
    pub use_clipboard: bool,
    pub auto_paste: bool,
}

#[derive(Clone)]
pub struct OutputBehavior {
    pub behavior: Arc<Mutex<BehaviorOptions>>,
    #[cfg(target_os = "macos")]
    pub front_app_before_paste: Arc<Mutex<Option<String>>>,
    pub log_callback: Arc<Mutex<Option<LogCallback>>>,
}

impl OutputBehavior {
    pub fn new(
        behavior: Arc<Mutex<BehaviorOptions>>,
        #[cfg(target_os = "macos")] front_app_before_paste: Arc<Mutex<Option<String>>>,
        log_callback: Arc<Mutex<Option<LogCallback>>>,
    ) -> Self {
        Self {
            behavior,
            #[cfg(target_os = "macos")]
            front_app_before_paste,
            log_callback,
        }
    }

    pub fn set_behavior_options(&self, use_clipboard: bool, auto_paste: bool) {
        *self.behavior.lock().unwrap() = BehaviorOptions {
            use_clipboard,
            auto_paste,
        };
    }

    pub fn remember_front_app(&self) {
        #[cfg(target_os = "macos")]
        {
            let current = macos_helpers::frontmost_bundle_id();
            *self.front_app_before_paste.lock().unwrap() = current;
        }
    }

    pub fn apply_output(&self, text: &str) {
        let behavior = *self.behavior.lock().unwrap();
        if behavior.auto_paste {
            // 1) Copy to clipboard
            Self::copy_to_clipboard_only(text, &self.log_callback);
            // 2) Auto-paste
            #[cfg(target_os = "macos")]
            macos_helpers::hide_app();
            #[cfg(target_os = "macos")]
            {
                if let Some(bid) = self.front_app_before_paste.lock().unwrap().clone() {
                    let _ = macos_helpers::activate_bundle_id(&bid);
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
                loop {
                    if let Some(front) = macos_helpers::frontmost_bundle_id() {
                        if front != "com.hootvoice.HootVoice" {
                            break;
                        }
                    }
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(40));
                }
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
            let ok = crate::utils::keyboard::auto_paste();
            if ok {
                Self::log_with_callback(&self.log_callback, "[Keyboard] Sent paste");
            } else {
                #[cfg(target_os = "macos")]
                {
                    Self::log_with_callback(&self.log_callback, "[Warning] Auto paste failed. Enable 'HootVoice' in System Settings → Privacy & Security → Accessibility, and allow 'System Events' under Automation.");
                }
                #[cfg(target_os = "linux")]
                {
                    Self::log_with_callback(&self.log_callback, "[Warning] Auto paste failed. On Wayland install 'wtype'; on X11 install 'xdotool'.");
                }
                #[cfg(target_os = "windows")]
                {
                    Self::log_with_callback(
                        &self.log_callback,
                        "[Warning] Auto paste failed. PowerShell SendKeys may be blocked.",
                    );
                }
            }
            crate::utils::sound::stop_loop("processing");
            crate::utils::sound::play_sound_async("sounds/complete.mp3");
        } else if behavior.use_clipboard {
            Self::copy_to_clipboard_only(text, &self.log_callback);
            crate::utils::sound::stop_loop("processing");
            crate::utils::sound::play_sound_async("sounds/complete.mp3");
        } else {
            Self::log_with_callback(
                &self.log_callback,
                "[Output] Clipboard output is disabled (settings)",
            );
        }
    }

    fn log_with_callback(log_callback: &Arc<Mutex<Option<LogCallback>>>, message: &str) {
        if let Some(ref callback) = *log_callback.lock().unwrap() {
            callback(message);
        }
        if let Some(rest) = message.strip_prefix("[Error]") {
            tracing::error!("{}", rest.trim());
        } else if let Some(rest) = message.strip_prefix("[Warning]") {
            tracing::warn!("{}", rest.trim());
        } else if let Some(rest) = message.strip_prefix("[Info]") {
            tracing::info!("{}", rest.trim());
        } else {
            tracing::info!("{}", message);
        }
    }

    // Copy to clipboard only (no auto‑paste/sounds)
    fn copy_to_clipboard_only(text: &str, log_callback: &Arc<Mutex<Option<LogCallback>>>) {
        use std::process::Command;

        #[cfg(target_os = "linux")]
        {
            match Command::new("xclip")
                .arg("-selection")
                .arg("clipboard")
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                Ok(mut child) => {
                    if let Some(stdin) = child.stdin.as_mut() {
                        use std::io::Write;
                        let _ = stdin.write_all(text.as_bytes());
                    }
                    let _ = child.wait();
                    Self::log_with_callback(log_callback, "[Clipboard] Copied text");
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Self::log_with_callback(
                            log_callback,
                            "[Warning] xclip not found; trying wl-copy",
                        );
                    } else {
                        Self::log_with_callback(
                            log_callback,
                            &format!("[Warning] Failed to run xclip: {e}"),
                        );
                    }
                    match Command::new("wl-copy")
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                    {
                        Ok(mut child) => {
                            if let Some(stdin) = child.stdin.as_mut() {
                                use std::io::Write;
                                let _ = stdin.write_all(text.as_bytes());
                            }
                            let _ = child.wait();
                            Self::log_with_callback(
                                log_callback,
                                "[Clipboard] Copied text (Wayland)",
                            );
                        }
                        Err(e2) => {
                            if e2.kind() == std::io::ErrorKind::NotFound {
                                Self::log_with_callback(
                                    log_callback,
                                    "[Warning] wl-copy not found; failed to copy to clipboard",
                                );
                            } else {
                                Self::log_with_callback(
                                    log_callback,
                                    &format!("[Warning] Failed to run wl-copy: {e2}"),
                                );
                            }
                        }
                    }
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            if macos_helpers::set_clipboard(text) {
                Self::log_with_callback(log_callback, "[Clipboard] Copied text");
            } else {
                let status = std::process::Command::new("/usr/bin/pbcopy")
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        use std::io::Write;
                        if let Some(stdin) = child.stdin.as_mut() {
                            let _ = stdin.write_all(text.as_bytes());
                        }
                        child.wait()
                    });
                match status {
                    Ok(st) if st.success() => {
                        Self::log_with_callback(log_callback, "[Clipboard] Copied text")
                    }
                    Ok(st) => Self::log_with_callback(
                        log_callback,
                        &format!(
                            "[Warning] Failed to copy to clipboard (pbcopy status={:?})",
                            st.code()
                        ),
                    ),
                    Err(e) => Self::log_with_callback(
                        log_callback,
                        &format!("[Warning] Failed to copy to clipboard (pbcopy err={})", e),
                    ),
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            // Use Windows Clipboard API via clipboard-win to ensure Unicode (CF_UNICODETEXT)
            // Avoid piping to `clip.exe` which expects the current ANSI/OEM codepage and causes mojibake for UTF‑8.
            match clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text) {
                Ok(()) => {
                    Self::log_with_callback(log_callback, "[Clipboard] Copied text");
                }
                Err(e) => {
                    // Fallback (best-effort): PowerShell Set-Clipboard with Unicode
                    let ps = Command::new("powershell")
                        .arg("-NoProfile")
                        .arg("-Command")
                        .arg(
                            // Use here-string with explicit Unicode output encoding
                            "[Console]::OutputEncoding=[Text.Encoding]::Unicode; $t=@\"\n"
                                .to_string()
                                + text
                                + "\n\"@; Set-Clipboard -Value $t -AsPlainText",
                        )
                        .status();
                    match ps {
                        Ok(st) if st.success() => {
                            Self::log_with_callback(
                                log_callback,
                                "[Clipboard] Copied text (PowerShell)",
                            );
                        }
                        Ok(st) => {
                            Self::log_with_callback(
                                log_callback,
                                &format!(
                                    "[Warning] Failed to copy to clipboard (powershell status={:?}, err from API={})",
                                    st.code(), e
                                ),
                            );
                        }
                        Err(e2) => {
                            Self::log_with_callback(
                                log_callback,
                                &format!(
                                    "[Warning] Failed to copy to clipboard (winapi err={}, powershell err={})",
                                    e, e2
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
}
