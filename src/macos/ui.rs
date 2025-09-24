#[cfg(target_os = "macos")]
pub fn show_already_running_alert() {
    use cocoa::appkit::{NSApp, NSApplication};
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSAutoreleasePool, NSString};
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let _pool = NSAutoreleasePool::new(nil);
        let _ = NSApp();

        let alert: id = msg_send![class!(NSAlert), alloc];
        let alert: id = msg_send![alert, init];
        let title = NSString::alloc(nil).init_str(&i18n::tr("alert-already-running-title"));
        let info = NSString::alloc(nil).init_str(&i18n::tr("alert-already-running-info"));
        let ok = NSString::alloc(nil).init_str(&i18n::tr("btn-ok"));
        let _: () = msg_send![alert, setMessageText: title];
        let _: () = msg_send![alert, setInformativeText: info];
        let _: id = msg_send![alert, addButtonWithTitle: ok];
        let _: i64 = msg_send![alert, runModal];
    }
}

#[cfg(target_os = "macos")]
pub fn try_activate_existing_instance() {
    use std::process::Command;
    // Best-effort: ask the already running app (by bundle id) to activate via Apple Events.
    // This may prompt for Automation permission once.
    let _ = Command::new("osascript")
        .args([
            "-e",
            "try\n tell application id \"com.hootvoice.HootVoice\" to activate\nend try",
        ])
        .status();
}

#[cfg(target_os = "macos")]
pub fn hide_app() {
    use cocoa::appkit::NSApp;
    use cocoa::base::nil;
    use cocoa::foundation::NSAutoreleasePool;
    use objc::{msg_send, sel, sel_impl};
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);
        let app = NSApp();
        let _: () = msg_send![app, hide: nil];
    }
}

#[cfg(target_os = "macos")]
pub fn set_clipboard(text: &str) -> bool {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let pb: id = msg_send![class!(NSPasteboard), generalPasteboard];
        let _: () = msg_send![pb, clearContents];
        let s = NSString::alloc(nil).init_str(text);
        let arr: id = msg_send![class!(NSArray), arrayWithObject: s];
        let ok: i64 = msg_send![pb, writeObjects: arr];
        ok != 0
    }
}
use crate::i18n;
