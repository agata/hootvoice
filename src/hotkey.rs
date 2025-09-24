use anyhow::Result;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Callback = Box<dyn Fn() + Send + 'static>;

pub struct HotkeyManager {
    manager: GlobalHotKeyManager,
    hotkeys: Vec<HotKey>,
    callbacks: Arc<Mutex<Vec<Callback>>>,
    last_trigger_time: Arc<Mutex<Option<Instant>>>,
}

impl HotkeyManager {
    pub fn new() -> Result<Self> {
        let manager = GlobalHotKeyManager::new()?;

        Ok(Self {
            manager,
            hotkeys: Vec::new(),
            callbacks: Arc::new(Mutex::new(Vec::new())),
            last_trigger_time: Arc::new(Mutex::new(None)),
        })
    }

    pub fn register_hotkey<F>(&mut self, hotkey_str: &str, callback: F) -> Result<()>
    where
        F: Fn() + Send + 'static,
    {
        let hotkey = self.parse_hotkey(hotkey_str)?;

        self.manager.register(hotkey)?;
        self.hotkeys.push(hotkey);

        let mut callbacks = self.callbacks.lock().unwrap();
        callbacks.push(Box::new(callback));

        Ok(())
    }

    // removed: unregister_all (not used)

    // removed: `handle_events` (unused)

    pub fn spawn_event_thread(&self) {
        let hotkeys = self.hotkeys.clone();
        let callbacks = Arc::clone(&self.callbacks);
        let last_trigger_time = Arc::clone(&self.last_trigger_time);
        std::thread::spawn(move || loop {
            if let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if event.state == global_hotkey::HotKeyState::Pressed {
                    let mut last_time = last_trigger_time.lock().unwrap();
                    let now = Instant::now();
                    if let Some(last) = *last_time {
                        if now.duration_since(last) < Duration::from_millis(500) {
                            println!("[Hotkey] Debounced - ignoring rapid trigger");
                            continue;
                        }
                    }
                    *last_time = Some(now);
                    drop(last_time);

                    if let Some(index) = hotkeys.iter().position(|h| h.id() == event.id) {
                        let callbacks = callbacks.lock().unwrap();
                        if let Some(callback) = callbacks.get(index) {
                            println!("[Hotkey] Executing callback (toggle recording)");
                            callback();
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        });
    }

    fn parse_hotkey(&self, hotkey_str: &str) -> Result<HotKey> {
        // Normalize: replace full-width '+' with '+' and trim spaces
        let normalized = hotkey_str.replace('＋', "+");
        let parts: Vec<String> = normalized
            .split('+')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let mut modifiers = Modifiers::empty();
        let mut key_code = None;

        for part in parts {
            match part.to_lowercase().as_str() {
                "ctrl" | "control" => modifiers.insert(Modifiers::CONTROL),
                "shift" => modifiers.insert(Modifiers::SHIFT),
                "alt" | "option" => modifiers.insert(Modifiers::ALT),
                "cmd" | "command" | "super" | "win" | "windows" => {
                    modifiers.insert(Modifiers::SUPER)
                }
                key => {
                    key_code = Some(self.parse_key_code(key)?);
                }
            }
        }

        let code = key_code.ok_or_else(|| anyhow::anyhow!("No key specified in hotkey"))?;

        Ok(HotKey::new(Some(modifiers), code))
    }

    fn parse_key_code(&self, key: &str) -> Result<Code> {
        Ok(match key.to_lowercase().as_str() {
            "a" => Code::KeyA,
            "b" => Code::KeyB,
            "c" => Code::KeyC,
            "d" => Code::KeyD,
            "e" => Code::KeyE,
            "f" => Code::KeyF,
            "g" => Code::KeyG,
            "h" => Code::KeyH,
            "i" => Code::KeyI,
            "j" => Code::KeyJ,
            "k" => Code::KeyK,
            "l" => Code::KeyL,
            "m" => Code::KeyM,
            "n" => Code::KeyN,
            "o" => Code::KeyO,
            "p" => Code::KeyP,
            "q" => Code::KeyQ,
            "r" => Code::KeyR,
            "s" => Code::KeyS,
            "t" => Code::KeyT,
            "u" => Code::KeyU,
            "v" => Code::KeyV,
            "w" => Code::KeyW,
            "x" => Code::KeyX,
            "y" => Code::KeyY,
            "z" => Code::KeyZ,
            "0" => Code::Digit0,
            "1" => Code::Digit1,
            "2" => Code::Digit2,
            "3" => Code::Digit3,
            "4" => Code::Digit4,
            "5" => Code::Digit5,
            "6" => Code::Digit6,
            "7" => Code::Digit7,
            "8" => Code::Digit8,
            "9" => Code::Digit9,
            "f1" => Code::F1,
            "f2" => Code::F2,
            "f3" => Code::F3,
            "f4" => Code::F4,
            "f5" => Code::F5,
            "f6" => Code::F6,
            "f7" => Code::F7,
            "f8" => Code::F8,
            "f9" => Code::F9,
            "f10" => Code::F10,
            "f11" => Code::F11,
            "f12" => Code::F12,
            "space" => Code::Space,
            "enter" | "return" => Code::Enter,
            "tab" => Code::Tab,
            "escape" | "esc" => Code::Escape,
            _ => return Err(anyhow::anyhow!("Unknown key: {}", key)),
        })
    }
}
