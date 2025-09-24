use std::process::Command;

#[cfg(target_os = "macos")]
mod macos_input {
    use std::os::raw::c_void;

    // CoreGraphics / AX FFI types
    type CGEventRef = *mut c_void;
    type CGEventSourceRef = *mut c_void;
    type CGEventFlags = u64; // 64-bit mask
    type CGKeyCode = u16; // UInt16
    type CGEventTapLocation = u32; // UInt32

    // CoreFoundation dictionary for AX prompt
    // Avoid fragile CFDictionary creation for AX prompt on macOS 15+; use simple trust check.
    // use core_foundation::base::{CFRelease as CFReleaseCF, CFTypeRef, TCFType};
    // use core_foundation::boolean::kCFBooleanTrue;
    // use core_foundation::dictionary::{CFDictionaryCreate, CFDictionaryRef};
    // use core_foundation::string::CFString;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        // CGEvent APIs
        fn CGEventCreateKeyboardEvent(
            source: CGEventSourceRef,
            virtualKey: CGKeyCode,
            keyDown: bool,
        ) -> CGEventRef;
        fn CGEventSetFlags(event: CGEventRef, flags: CGEventFlags);
        fn CGEventPost(tap: CGEventTapLocation, event: CGEventRef);
        fn CGEventSourceCreate(state_id: u32) -> CGEventSourceRef; // CGEventSourceStateID
        fn CFRelease(cf: *const c_void);

        // AX (Accessibility) trust check
        fn AXIsProcessTrusted() -> bool;
        // fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    }

    // Prefer posting at HID level for synthesized events
    const KCG_HID_EVENT_TAP: CGEventTapLocation = 0; // kCGHIDEventTap
    const KCG_EVENT_FLAG_MASK_COMMAND: CGEventFlags = 1 << 20; // kCGEventFlagMaskCommand

    // Virtual key codes (ANSI layout)
    const KEY_V: CGKeyCode = 9; // kVK_ANSI_V

    fn ax_is_trusted_prompt(_prompt: bool) -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    pub fn send_cmd_v() -> bool {
        // Ensure AX permission; prompt on first call
        if !ax_is_trusted_prompt(true) {
            eprintln!("[auto_paste] Accessibility permission not granted.");
            return false;
        }

        unsafe {
            // Use HID system source so events are widely accepted
            let src = CGEventSourceCreate(1); // kCGEventSourceStateHIDSystemState
            if src.is_null() {
                eprintln!("[auto_paste] CGEventSourceCreate failed");
                return false;
            }

            // Key down (V) with Command
            let v_down = CGEventCreateKeyboardEvent(src, KEY_V, true);
            if v_down.is_null() {
                CFRelease(src);
                return false;
            }
            CGEventSetFlags(v_down, KCG_EVENT_FLAG_MASK_COMMAND);
            CGEventPost(KCG_HID_EVENT_TAP, v_down);
            CFRelease(v_down as *const c_void);

            std::thread::sleep(std::time::Duration::from_millis(6));

            // Key up (V) with Command
            let v_up = CGEventCreateKeyboardEvent(src, KEY_V, false);
            if v_up.is_null() {
                CFRelease(src);
                return false;
            }
            CGEventSetFlags(v_up, KCG_EVENT_FLAG_MASK_COMMAND);
            CGEventPost(KCG_HID_EVENT_TAP, v_up);
            CFRelease(v_up as *const c_void);

            CFRelease(src);
            true
        }
    }
}

/// Auto-paste feature (send Ctrl/Cmd+V)
/// Returns true if any method succeeds
pub fn auto_paste() -> bool {
    // 1. Wayland: try wtype
    if std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE").unwrap_or_default() == "wayland"
    {
        // Send Ctrl+V via wtype
        if Command::new("wtype")
            .args(["-M", "ctrl", "-k", "v", "-m", "ctrl"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
    }

    // 2. X11: try xdotool
    if std::env::var("DISPLAY").is_ok() {
        // Send Ctrl+V via xdotool
        if Command::new("xdotool")
            .args(["key", "ctrl+v"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }

        // Try ydotool (may require root)
        if Command::new("ydotool")
            .args(["key", "29:1", "47:1", "47:0", "29:0"]) // Ctrl+V
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
    }

    // 3. macOS: use osascript (fallback if CGEvent not available)
    #[cfg(target_os = "macos")]
    {
        // Prefer CGEvent (requires Accessibility permission)
        if macos_input::send_cmd_v() {
            return true;
        }
        // Fallback: AppleScript (may be blocked by environment)
        match Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "tell application \"System Events\" to keystroke \"v\" using command down",
            ])
            .output()
        {
            Ok(out) if out.status.success() => {
                return true;
            }
            Ok(out) => {
                eprintln!(
                    "[auto_paste] osascript failed: status={:?}, stderr={}",
                    out.status.code(),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            Err(e) => {
                eprintln!("[auto_paste] failed to invoke osascript: {}", e);
            }
        }
    }

    // 4. Windows: use PowerShell
    #[cfg(target_os = "windows")]
    {
        if Command::new("powershell")
            .args(["-Command", "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('^v')"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
    }

    false
}
