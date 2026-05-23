use crate::ime::InputMode;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};

pub struct WindowsImeController;

pub struct WindowsSingleInstance {
    handle: HANDLE,
}

impl WindowsImeController {
    pub fn new() -> Self {
        Self
    }

    pub fn current_mode(&self) -> Result<InputMode, String> {
        let target_hwnd = current_ime_target_hwnd()?;
        read_ime_mode(target_hwnd)
    }

    pub fn switch_to(&self, mode: InputMode) -> Result<(), String> {
        let target_hwnd = current_ime_target_hwnd()?;
        write_ime_mode(target_hwnd, mode)
    }
}

impl WindowsSingleInstance {
    pub fn acquire(name: &str) -> Result<Self, String> {
        let mutex_name = wide_null(name);
        let handle = unsafe { CreateMutexW(null_mut(), 0, PCWSTR(mutex_name.as_ptr())) };
        if handle.is_null() {
            return Err(format!("CreateMutexW failed: {}", unsafe { GetLastError().0 }));
        }
        let last_error = unsafe { GetLastError() };
        if last_error == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err("smart-shift is already running".to_string());
        }

        Ok(Self { handle })
    }
}

impl Drop for WindowsSingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

pub fn current_ime_target_hwnd() -> Result<HWND, String> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return Err("GetForegroundWindow returned null".to_string());
    }

    let mut process_id = 0u32;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) };
    if thread_id == 0 {
        return Err("GetWindowThreadProcessId failed".to_string());
    }

    let mut gui_info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..GUITHREADINFO::default()
    };
    let ok = unsafe { GetGUIThreadInfo(thread_id, &mut gui_info) };
    if ok == 0 {
        return Err("GetGUIThreadInfo failed".to_string());
    }

    Ok(resolve_ime_target_hwnd(hwnd, gui_info.hwndFocus))
}

fn resolve_ime_target_hwnd(foreground_hwnd: HWND, focus_hwnd: HWND) -> HWND {
    if focus_hwnd.is_null() {
        foreground_hwnd
    } else {
        focus_hwnd
    }
}

pub fn read_ime_mode(target_hwnd: HWND) -> Result<InputMode, String> {
    let is_open = read_ime_open_status(target_hwnd)?;
    if !is_open {
        return Ok(InputMode::English);
    }

    match read_ime_conversion_status_via_default_window(target_hwnd)
        .or_else(|_| read_ime_conversion_status(target_hwnd))
    {
        Ok(conversion_mode) => Ok(input_mode_from_conversion_status(conversion_mode)),
        Err(_) => Ok(InputMode::Chinese),
    }
}

pub fn write_ime_mode(target_hwnd: HWND, mode: InputMode) -> Result<(), String> {
    let desired_open = ime_open_status_for_mode(mode);
    let desired_conversion_mode = ime_conversion_status_for_mode(mode);
    let mut last_error = None;

    match with_ime_context(target_hwnd, |himc| {
        let set_ok = unsafe { ImmSetOpenStatus(himc, desired_open as i32) };
        if set_ok == 0 {
            return Err(format!(
                "ImmSetOpenStatus failed for hwnd=0x{:X} mode={mode}",
                target_hwnd as isize
            ));
        }
        Ok(())
    }) {
        Ok(()) => {
            if verify_ime_mode(target_hwnd, mode).is_ok() {
                return Ok(());
            }
        }
        Err(error) => last_error = Some(error),
    }

    match write_ime_conversion_status_via_default_window(target_hwnd, desired_conversion_mode) {
        Ok(()) => {
            if verify_ime_mode(target_hwnd, mode).is_ok() {
                return Ok(());
            }
        }
        Err(error) => last_error = Some(error),
    }

    match write_ime_conversion_status(target_hwnd, desired_conversion_mode) {
        Ok(()) => {
            if verify_ime_mode(target_hwnd, mode).is_ok() {
                return Ok(());
            }
        }
        Err(error) => last_error = Some(error),
    }

    match write_ime_open_status_via_default_window(target_hwnd, desired_open) {
        Ok(()) => {
            if verify_ime_mode(target_hwnd, mode).is_ok() {
                Ok(())
            } else {
                Err(format!(
                    "IME mode verification failed after WM_IME_CONTROL for hwnd=0x{:X}",
                    target_hwnd as isize
                ))
            }
        }
        Err(error) => {
            if verify_ime_mode(target_hwnd, mode).is_ok() {
                Ok(())
            } else {
                let context = last_error
                    .map(|previous| format!("; previous_error={previous}"))
                    .unwrap_or_default();
                Err(format!("{error}{context}"))
            }
        }
    }
}

fn read_ime_open_status(target_hwnd: HWND) -> Result<bool, String> {
    if let Ok(is_open) = with_ime_context(target_hwnd, |himc| {
        Ok(unsafe { ImmGetOpenStatus(himc) != 0 })
    }) {
        return Ok(is_open);
    }

    read_ime_open_status_via_default_window(target_hwnd)
}

fn read_ime_open_status_via_default_window(target_hwnd: HWND) -> Result<bool, String> {
    let default_ime_hwnd = unsafe { ImmGetDefaultIMEWnd(target_hwnd) };
    if default_ime_hwnd.is_null() {
        return Err(format!(
            "ImmGetContext returned null and ImmGetDefaultIMEWnd returned null for hwnd=0x{:X}",
            target_hwnd as isize
        ));
    }

    let result = unsafe { SendMessageW(default_ime_hwnd, WM_IME_CONTROL, IMC_GETOPENSTATUS, 0) };
    Ok(result != 0)
}

fn write_ime_open_status_via_default_window(
    target_hwnd: HWND,
    desired_open: bool,
) -> Result<(), String> {
    let default_ime_hwnd = unsafe { ImmGetDefaultIMEWnd(target_hwnd) };
    if default_ime_hwnd.is_null() {
        return Err(format!(
            "ImmGetContext returned null and ImmGetDefaultIMEWnd returned null for hwnd=0x{:X}",
            target_hwnd as isize
        ));
    }

    let result = unsafe {
        SendMessageW(
            default_ime_hwnd,
            WM_IME_CONTROL,
            IMC_SETOPENSTATUS,
            desired_open as isize,
        )
    };
    if result == 0 {
        return Err(format!(
            "WM_IME_CONTROL IMC_SETOPENSTATUS failed for hwnd=0x{:X}",
            target_hwnd as isize
        ));
    }

    Ok(())
}

fn read_ime_conversion_status_via_default_window(target_hwnd: HWND) -> Result<Dword, String> {
    let default_ime_hwnd = unsafe { ImmGetDefaultIMEWnd(target_hwnd) };
    if default_ime_hwnd.is_null() {
        return Err(format!(
            "ImmGetDefaultIMEWnd returned null for hwnd=0x{:X}",
            target_hwnd as isize
        ));
    }

    let result =
        unsafe { SendMessageW(default_ime_hwnd, WM_IME_CONTROL, IMC_GETCONVERSIONMODE, 0) };
    Ok(result as Dword)
}

fn write_ime_conversion_status_via_default_window(
    target_hwnd: HWND,
    desired_conversion_mode: Dword,
) -> Result<(), String> {
    let default_ime_hwnd = unsafe { ImmGetDefaultIMEWnd(target_hwnd) };
    if default_ime_hwnd.is_null() {
        return Err(format!(
            "ImmGetDefaultIMEWnd returned null for hwnd=0x{:X}",
            target_hwnd as isize
        ));
    }

    let result = unsafe {
        SendMessageW(
            default_ime_hwnd,
            WM_IME_CONTROL,
            IMC_SETCONVERSIONMODE,
            desired_conversion_mode as LPARAM,
        )
    };
    if result == 0 {
        return Err(format!(
            "WM_IME_CONTROL IMC_SETCONVERSIONMODE failed for hwnd=0x{:X}",
            target_hwnd as isize
        ));
    }

    Ok(())
}

fn read_ime_conversion_status(target_hwnd: HWND) -> Result<Dword, String> {
    with_ime_context(target_hwnd, |himc| {
        let mut conversion = 0u32;
        let mut sentence = 0u32;
        let ok = unsafe { ImmGetConversionStatus(himc, &mut conversion, &mut sentence) };
        if ok == 0 {
            return Err(format!(
                "ImmGetConversionStatus failed for hwnd=0x{:X}",
                target_hwnd as isize
            ));
        }

        Ok(conversion)
    })
}

fn write_ime_conversion_status(
    target_hwnd: HWND,
    desired_conversion_mode: Dword,
) -> Result<(), String> {
    with_ime_context(target_hwnd, |himc| {
        let mut conversion = 0u32;
        let mut sentence = 0u32;
        let get_ok = unsafe { ImmGetConversionStatus(himc, &mut conversion, &mut sentence) };
        if get_ok == 0 {
            return Err(format!(
                "ImmGetConversionStatus failed for hwnd=0x{:X}",
                target_hwnd as isize
            ));
        }

        let next_conversion =
            (conversion & !IME_CMODE_NATIVE) | (desired_conversion_mode & IME_CMODE_NATIVE);
        let set_ok = unsafe { ImmSetConversionStatus(himc, next_conversion, sentence) };
        if set_ok == 0 {
            return Err(format!(
                "ImmSetConversionStatus failed for hwnd=0x{:X}",
                target_hwnd as isize
            ));
        }

        Ok(())
    })
}

fn verify_ime_mode(target_hwnd: HWND, expected_mode: InputMode) -> Result<(), String> {
    match read_ime_mode(target_hwnd) {
        Ok(mode) if mode == expected_mode => Ok(()),
        Ok(mode) => Err(format!(
            "IME mode verification failed for hwnd=0x{:X}: expected={expected_mode} actual={mode}",
            target_hwnd as isize
        )),
        Err(error) => Err(format!("IME mode verification read failed: {error}")),
    }
}

fn with_ime_context<T, F>(target_hwnd: HWND, operation: F) -> Result<T, String>
where
    F: FnOnce(*mut c_void) -> Result<T, String>,
{
    let himc = unsafe { ImmGetContext(target_hwnd) };
    if himc.is_null() {
        return Err(format!(
            "ImmGetContext returned null for hwnd=0x{:X}",
            target_hwnd as isize
        ));
    }

    let result = operation(himc);
    let release_ok = unsafe { ImmReleaseContext(target_hwnd, himc) };
    if release_ok == 0 {
        return Err(format!(
            "ImmReleaseContext failed for hwnd=0x{:X}",
            target_hwnd as isize
        ));
    }

    result
}

fn ime_open_status_for_mode(mode: InputMode) -> bool {
    match mode {
        InputMode::Chinese => true,
        InputMode::English => false,
    }
}

fn ime_conversion_status_for_mode(mode: InputMode) -> Dword {
    match mode {
        InputMode::Chinese => IME_CMODE_NATIVE,
        InputMode::English => 0,
    }
}

fn input_mode_from_conversion_status(conversion_mode: Dword) -> InputMode {
    if conversion_mode & IME_CMODE_NATIVE != 0 {
        InputMode::Chinese
    } else {
        InputMode::English
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

// -- Type aliases --
type Bool = i32;
type Dword = u32;
type Hwnd = *mut c_void;
type HWND = Hwnd;
type Handle = *mut c_void;
type HANDLE = Handle;
type Lparam = isize;
type LPARAM = Lparam;
type Wparam = usize;
type WPARAM = Wparam;
type Lresult = isize;
type LRESULT = Lresult;
type UINT = u32;

// -- Constants --
const WM_IME_CONTROL: UINT = 0x0283;
const IMC_GETCONVERSIONMODE: WPARAM = 0x0001;
const IMC_SETCONVERSIONMODE: WPARAM = 0x0002;
const IMC_GETOPENSTATUS: WPARAM = 0x0005;
const IMC_SETOPENSTATUS: WPARAM = 0x0006;
const IME_CMODE_NATIVE: Dword = 0x0001;

// -- Structs --
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_snake_case)]
struct GUITHREADINFO {
    cbSize: Dword,
    flags: Dword,
    hwndActive: HWND,
    hwndFocus: HWND,
    hwndCapture: HWND,
    hwndMenuOwner: HWND,
    hwndMoveSize: HWND,
    hwndCaret: HWND,
    rcCaret: Rect,
}

impl Default for GUITHREADINFO {
    fn default() -> Self {
        Self {
            cbSize: 0,
            flags: 0,
            hwndActive: null_mut(),
            hwndFocus: null_mut(),
            hwndCapture: null_mut(),
            hwndMenuOwner: null_mut(),
            hwndMoveSize: null_mut(),
            hwndCaret: null_mut(),
            rcCaret: Rect::default(),
        }
    }
}

// -- FFI --
#[link(name = "user32")]
unsafe extern "system" {
    fn GetForegroundWindow() -> HWND;
    fn GetWindowThreadProcessId(hWnd: HWND, lpdwProcessId: *mut Dword) -> Dword;
    fn GetGUIThreadInfo(idThread: Dword, lpgui: *mut GUITHREADINFO) -> Bool;
    fn SendMessageW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(
        lpMutexAttributes: *mut c_void,
        bInitialOwner: Bool,
        lpName: PCWSTR,
    ) -> HANDLE;
    fn CloseHandle(hObject: HANDLE) -> Bool;
}

#[link(name = "imm32")]
unsafe extern "system" {
    fn ImmGetDefaultIMEWnd(hWnd: HWND) -> HWND;
    fn ImmGetContext(hWnd: HWND) -> *mut c_void;
    fn ImmGetOpenStatus(hIMC: *mut c_void) -> Bool;
    fn ImmGetConversionStatus(
        hIMC: *mut c_void,
        lpfdwConversion: *mut Dword,
        lpfdwSentence: *mut Dword,
    ) -> Bool;
    fn ImmReleaseContext(hWnd: HWND, hIMC: *mut c_void) -> Bool;
    fn ImmSetConversionStatus(hIMC: *mut c_void, fdwConversion: Dword, fdwSentence: Dword) -> Bool;
    fn ImmSetOpenStatus(hIMC: *mut c_void, fOpen: Bool) -> Bool;
}
