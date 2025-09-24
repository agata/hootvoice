#[cfg(target_os = "macos")]
pub fn setup_menubar(app_name: &str) {
    use cocoa::appkit::{
        NSApp, NSApplication, NSApplicationActivationPolicy, NSEventModifierFlags, NSMenu,
        NSMenuItem,
    };
    use cocoa::base::{id, nil, YES};
    use cocoa::foundation::{NSAutoreleasePool, NSString};
    use objc::runtime::Sel;

    unsafe {
        // Ensure we have an autorelease pool while constructing menus
        let _pool = NSAutoreleasePool::new(nil);
        let app = NSApp();
        // Regular app activation so menubar works normally
        app.setActivationPolicy_(
            NSApplicationActivationPolicy::NSApplicationActivationPolicyRegular,
        );

        // Main menu
        let main_menu: id = NSMenu::new(nil).autorelease();

        // App menu (Quit)
        let app_menu_item: id = NSMenuItem::new(nil).autorelease();
        main_menu.addItem_(app_menu_item);
        let app_menu: id = NSMenu::alloc(nil)
            .initWithTitle_(NSString::alloc(nil).init_str(app_name))
            .autorelease();
        // Title is set on the submenu; menubar reflects submenu title

        let quit_title = format!("Quit {}", app_name);
        let quit_item: id = NSMenuItem::alloc(nil)
            .initWithTitle_action_keyEquivalent_(
                NSString::alloc(nil).init_str(&quit_title),
                Sel::register("terminate:"),
                NSString::alloc(nil).init_str("q"),
            )
            .autorelease();
        app_menu.addItem_(quit_item);
        app_menu_item.setSubmenu_(app_menu);

        // Window menu
        let window_menu_item: id = NSMenuItem::new(nil).autorelease();
        main_menu.addItem_(window_menu_item);
        let window_menu: id = NSMenu::alloc(nil)
            .initWithTitle_(NSString::alloc(nil).init_str("Window"))
            .autorelease();
        // Title is set on the submenu; menubar reflects submenu title

        // Minimize (Cmd+M)
        let minimize: id = NSMenuItem::alloc(nil)
            .initWithTitle_action_keyEquivalent_(
                NSString::alloc(nil).init_str("Minimize"),
                Sel::register("performMiniaturize:"),
                NSString::alloc(nil).init_str("m"),
            )
            .autorelease();
        window_menu.addItem_(minimize);

        // Zoom (no key)
        let zoom: id = NSMenuItem::alloc(nil)
            .initWithTitle_action_keyEquivalent_(
                NSString::alloc(nil).init_str("Zoom"),
                Sel::register("performZoom:"),
                NSString::alloc(nil).init_str(""),
            )
            .autorelease();
        window_menu.addItem_(zoom);

        // Enter Full Screen (Ctrl+Cmd+F)
        let fs: id = NSMenuItem::alloc(nil)
            .initWithTitle_action_keyEquivalent_(
                NSString::alloc(nil).init_str("Enter Full Screen"),
                Sel::register("toggleFullScreen:"),
                NSString::alloc(nil).init_str("f"),
            )
            .autorelease();
        // Add modifiers: Cmd + Control
        fs.setKeyEquivalentModifierMask_(
            NSEventModifierFlags::NSCommandKeyMask | NSEventModifierFlags::NSControlKeyMask,
        );
        window_menu.addItem_(fs);

        // Bring All to Front
        let front: id = NSMenuItem::alloc(nil)
            .initWithTitle_action_keyEquivalent_(
                NSString::alloc(nil).init_str("Bring All to Front"),
                Sel::register("arrangeInFront:"),
                NSString::alloc(nil).init_str(""),
            )
            .autorelease();
        window_menu.addItem_(front);

        window_menu_item.setSubmenu_(window_menu);

        app.setMainMenu_(main_menu);
        // Make this the app's Windows menu so system behaviors (Window menu in menu bar) work correctly
        app.setWindowsMenu_(window_menu);
        // Bring the app to front so the menu becomes active
        app.activateIgnoringOtherApps_(YES);
    }
}
