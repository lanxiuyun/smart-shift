use crate::ime::InputMode;
use crate::platform::windows::WindowsImeController;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

pub struct StartupCheckResult {
    pub ime_readable: bool,
    pub uia_available: bool,
    pub errors: Vec<String>,
}

pub fn run_startup_checks() -> StartupCheckResult {
    let mut errors = Vec::new();

    // Check IME read
    let ime_readable = match check_ime_read() {
        Ok(mode) => {
            println!("Startup check: IME readable, current mode = {mode}");
            true
        }
        Err(e) => {
            errors.push(format!("IME read failed: {e}"));
            false
        }
    };

    // Check UIA availability
    let uia_available = match check_uia_available() {
        Ok(()) => {
            println!("Startup check: UIA available");
            true
        }
        Err(e) => {
            errors.push(format!("UIA unavailable: {e}"));
            false
        }
    };

    StartupCheckResult {
        ime_readable,
        uia_available,
        errors,
    }
}

fn check_ime_read() -> Result<InputMode, String> {
    let controller = WindowsImeController::new();
    controller.current_mode()
}

fn check_uia_available() -> Result<(), String> {
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            return Err("COM initialization failed".to_string());
        }

        let result = CoCreateInstance::<_, IUIAutomation>(
            &CUIAutomation,
            None,
            CLSCTX_INPROC_SERVER,
        )
        .map(|_| ())
        .map_err(|e| format!("UIAutomation creation failed: {e}"));

        CoUninitialize();
        result
    }
}
