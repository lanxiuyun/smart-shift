use crate::classifier::{classify, Decision, DecisionReason};
use crate::config::AppConfig;
use crate::context::LineContext;
use crate::ime::InputMode;
use crate::logger::EventLogger;
use std::sync::RwLock;
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::Emitter;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND as WinHwnd};

use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};

use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationTextRange, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start,
    TextUnit_Line, UIA_TextPatternId,
};

use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

const STYLE_RESET: &str = "\x1b[0m";
const STYLE_BOLD: &str = "\x1b[1m";
const COLOR_DIM: &str = "\x1b[2m";
const COLOR_RED: &str = "\x1b[31m";
const COLOR_GREEN: &str = "\x1b[32m";
const COLOR_YELLOW: &str = "\x1b[33m";
const COLOR_CYAN: &str = "\x1b[36m";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppAdapter {
    ChromiumUia,
}

pub struct WindowsImeController;

pub struct WindowsSingleInstance {
    handle: HANDLE,
}

pub struct TrayRuntimeControl {
    stop_requested: AtomicBool,
    paused: AtomicBool,
}

impl TrayRuntimeControl {
    pub fn new() -> Self {
        Self {
            stop_requested: AtomicBool::new(false),
            paused: AtomicBool::new(false),
        }
    }

    #[allow(dead_code)]
    pub fn request_stop(&self) {
        self.stop_requested.store(true, Ordering::Relaxed);
    }

    pub fn stop_requested(&self) -> bool {
        self.stop_requested.load(Ordering::Relaxed)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn toggle_paused(&self) -> bool {
        let new_state = !self.is_paused();
        self.paused.store(new_state, Ordering::Relaxed);
        new_state
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }
}

impl WindowsSingleInstance {
    pub fn acquire(name: &str) -> Result<Self, String> {
        let mutex_name = wide_null(name);
        let handle = unsafe { CreateMutexW(null_mut(), 0, PCWSTR(mutex_name.as_ptr())) };
        if handle.is_null() {
            return Err(format!("CreateMutexW failed: {}", unsafe {
                GetLastError().0
            }));
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

#[derive(Clone, serde::Serialize)]
pub struct WatcherEvent {
    pub line_text: String,
    pub source: String,
    pub current_mode: Option<String>,
    pub target_mode: Option<String>,
    pub switched: bool,
    pub preserved: bool,
    pub reason: String,
    pub error: Option<String>,
    pub debug: bool,
    pub window_title: Option<String>,
    pub process_name: Option<String>,
    pub focus_class: Option<String>,
    pub ime_error: Option<String>,
    pub caret: Option<String>,
    pub doc_len: Option<usize>,
    pub selection_start: Option<usize>,
    pub selection_end: Option<usize>,
    pub line_number: Option<usize>,
    pub cursor_utf16: Option<usize>,
    pub cursor_chars: Option<usize>,
}

pub struct ForegroundWatcher {
    interval: Duration,
    debug: Arc<AtomicBool>,
    app_config: Arc<RwLock<AppConfig>>,
    app_handle: Option<tauri::AppHandle>,
    event_logger: Option<Arc<EventLogger>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotTransition {
    Emit,
    Ignore,
    SuppressedTextEdit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatcherSwitchOutcome {
    Applied,
    SkippedAlreadyMatched,
    SkippedUnknownCurrentMode,
}

impl ForegroundWatcher {
    pub fn new(
        interval_ms: u64,
        debug: Arc<AtomicBool>,
        app_config: Arc<RwLock<AppConfig>>,
        app_handle: Option<tauri::AppHandle>,
        event_logger: Option<Arc<EventLogger>>,
    ) -> Self {
        Self {
            interval: Duration::from_millis(interval_ms.max(50)),
            debug,
            app_config,
            app_handle,
            event_logger,
        }
    }

    #[allow(dead_code)]
    pub fn run(&self) -> Result<(), String> {
        self.run_until(|| false)
    }

    #[allow(dead_code)]
    pub fn run_until_stopped(&self, stop_signal: &AtomicBool) -> Result<(), String> {
        self.run_until(|| stop_signal.load(Ordering::Relaxed))
    }

    pub fn run_until_controlled(&self, control: &TrayRuntimeControl) -> Result<(), String> {
        println!(
            "{}smart-shift watcher started{}  polling={}ms  debug={}",
            STYLE_BOLD,
            STYLE_RESET,
            self.interval.as_millis(),
            self.debug.load(Ordering::Relaxed)
        );

        let mut last_snapshot: Option<ForegroundSnapshot> = None;
        let mut last_error: Option<String> = None;
        let mut suppressed_text_edit = false;
        let mut was_paused = false;

        while !control.stop_requested() {
            if control.is_paused() {
                was_paused = true;
                thread::sleep(self.interval);
                continue;
            }

            if was_paused {
                last_snapshot = None;
                last_error = None;
                suppressed_text_edit = false;
                was_paused = false;
            }

            match capture_foreground_snapshot() {
                Ok(snapshot) => {
                    last_error = None;

                    // Check blacklist / whitelist
                    if let Ok(cfg) = self.app_config.read() {
                        if !cfg.is_app_allowed(&snapshot.process_name) {
                            last_snapshot = Some(snapshot);
                            thread::sleep(self.interval);
                            continue;
                        }
                    }

                    // Skip IME switching if the user is actively composing (pinyin input, etc.)
                    if is_ime_composing(snapshot.focus_hwnd as HWND) {
                        last_snapshot = Some(snapshot);
                        thread::sleep(self.interval);
                        continue;
                    }

                    let transition = classify_snapshot_transition_with_state(
                        last_snapshot.as_ref(),
                        &snapshot,
                        suppressed_text_edit,
                    );
                    if transition == SnapshotTransition::Emit {
                        print_snapshot(
                            &snapshot,
                            self.debug.load(Ordering::Relaxed),
                            &self.app_handle,
                            self.event_logger.as_deref(),
                        );
                    }
                    suppressed_text_edit = transition == SnapshotTransition::SuppressedTextEdit;
                    last_snapshot = Some(snapshot);
                }
                Err(error) => {
                    if last_error.as_ref() != Some(&error) {
                        println!();
                        println!("{}! watcher warning:{} {error}", COLOR_RED, STYLE_RESET);
                        last_error = Some(error);
                    }
                }
            }
            thread::sleep(self.interval);
        }

        Ok(())
    }

    #[allow(dead_code)]
    fn run_until<F>(&self, should_stop: F) -> Result<(), String>
    where
        F: Fn() -> bool,
    {
        println!(
            "{}smart-shift watcher started{}  polling={}ms  debug={}",
            STYLE_BOLD,
            STYLE_RESET,
            self.interval.as_millis(),
            self.debug.load(Ordering::Relaxed)
        );

        let mut last_snapshot: Option<ForegroundSnapshot> = None;
        let mut last_error: Option<String> = None;
        let mut suppressed_text_edit = false;

        while !should_stop() {
            match capture_foreground_snapshot() {
                Ok(snapshot) => {
                    last_error = None;

                    // Skip IME switching if the user is actively composing
                    if is_ime_composing(snapshot.focus_hwnd as HWND) {
                        last_snapshot = Some(snapshot);
                        thread::sleep(self.interval);
                        continue;
                    }

                    let transition = classify_snapshot_transition_with_state(
                        last_snapshot.as_ref(),
                        &snapshot,
                        suppressed_text_edit,
                    );
                    if transition == SnapshotTransition::Emit {
                        print_snapshot(
                            &snapshot,
                            self.debug.load(Ordering::Relaxed),
                            &self.app_handle,
                            self.event_logger.as_deref(),
                        );
                    }
                    suppressed_text_edit = transition == SnapshotTransition::SuppressedTextEdit;
                    last_snapshot = Some(snapshot);
                }
                Err(error) => {
                    if last_error.as_ref() != Some(&error) {
                        println!();
                        println!("{}! watcher warning:{} {error}", COLOR_RED, STYLE_RESET);
                        last_error = Some(error);
                    }
                }
            }
            thread::sleep(self.interval);
        }

        Ok(())
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[allow(dead_code)]
pub fn show_error_dialog(title: &str, message: &str) {
    let title = wide_null(title);
    let message = wide_null(message);
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn classify_snapshot_transition_with_state(
    previous: Option<&ForegroundSnapshot>,
    current: &ForegroundSnapshot,
    suppressed_text_edit: bool,
) -> SnapshotTransition {
    let Some(previous) = previous else {
        return SnapshotTransition::Emit;
    };

    if previous.foreground_hwnd != current.foreground_hwnd
        || previous.foreground_title != current.foreground_title
        || previous.focus_hwnd != current.focus_hwnd
        || previous.focus_class != current.focus_class
    {
        return SnapshotTransition::Emit;
    }

    let previous_text = &previous.text_snapshot;
    let current_text = &current.text_snapshot;
    let cursor_changed = previous_text.selection_start_utf16 != current_text.selection_start_utf16
        || previous_text.selection_end_utf16 != current_text.selection_end_utf16
        || previous_text.line_index != current_text.line_index
        || previous_text.line_cursor_utf16 != current_text.line_cursor_utf16
        || previous_text.line_cursor_chars != current_text.line_cursor_chars;
    let caret_changed = previous.caret_hwnd != current.caret_hwnd
        || previous.caret_left != current.caret_left
        || previous.caret_top != current.caret_top
        || previous.caret_right != current.caret_right
        || previous.caret_bottom != current.caret_bottom;

    if previous_text.source != current_text.source {
        return SnapshotTransition::Emit;
    }

    if previous_text.document_len_utf16 != current_text.document_len_utf16 {
        if looks_like_text_edit(previous_text, current_text) {
            return SnapshotTransition::SuppressedTextEdit;
        }

        if previous_text.line_index != current_text.line_index
            && (cursor_changed || caret_changed)
        {
            return SnapshotTransition::Emit;
        }

        // Chromium UIA can report fluctuating document lengths due to ghost character
        // noise even when no actual edit occurred. If the line text is unchanged and
        // the cursor has moved, treat it as a genuine cursor relocation.
        if previous_text.line_text == current_text.line_text
            && (cursor_changed || caret_changed)
        {
            return SnapshotTransition::Emit;
        }

        return SnapshotTransition::Ignore;
    }

    if suppressed_text_edit && looks_like_newline_followup(previous_text, current_text) {
        return SnapshotTransition::Ignore;
    }

    if previous_text.line_text != current_text.line_text {
        // Detect typing: if line text changed in a way consistent with inserting
        // a character at the cursor position, suppress the switch.
        if looks_like_typing(previous_text, current_text) {
            return SnapshotTransition::Ignore;
        }

        // Detect IME composition: if the previous line text is a prefix of the current
        // line text and the inserted part is lowercase ASCII (pinyin input), suppress.
        if looks_like_ime_composition(previous_text, current_text) {
            return SnapshotTransition::Ignore;
        }

        return if looks_like_blank_line_input_edit(previous_text, current_text) {
            SnapshotTransition::Ignore
        } else if cursor_changed || caret_changed {
            SnapshotTransition::Emit
        } else {
            SnapshotTransition::Ignore
        };
    }

    if cursor_changed || caret_changed {
        // Detect UIA lag where cursor jumps from end-of-line to start but line_index didn't update.
        // This happens in some editors when moving across a newline; ignoring prevents
        // re-classifying the old line content after the cursor has already left.
        if previous_text.line_index == current_text.line_index
            && previous_text.line_text == current_text.line_text
            && previous_text.line_cursor_chars == previous_text.line_text.chars().count()
            && current_text.line_cursor_chars == 0
        {
            return SnapshotTransition::Ignore;
        }
        SnapshotTransition::Emit
    } else {
        SnapshotTransition::Ignore
    }
}

fn looks_like_text_edit(previous: &TextSnapshot, current: &TextSnapshot) -> bool {
    let document_delta = current.document_len_utf16 as isize - previous.document_len_utf16 as isize;
    let selection_start_delta =
        current.selection_start_utf16 as isize - previous.selection_start_utf16 as isize;
    let selection_end_delta =
        current.selection_end_utf16 as isize - previous.selection_end_utf16 as isize;

    selection_start_delta == document_delta && selection_end_delta == document_delta
}

fn looks_like_newline_followup(previous: &TextSnapshot, current: &TextSnapshot) -> bool {
    if current.line_index != previous.line_index + 1 {
        return false;
    }

    if current.line_cursor_utf16 != 0 || current.line_cursor_chars != 0 {
        return false;
    }

    if !current.line_text.trim().is_empty() {
        return false;
    }

    let previous_line_len_utf16 = previous.line_text.encode_utf16().count();
    let previous_line_len_chars = previous.line_text.chars().count();

    previous.line_cursor_utf16 == previous_line_len_utf16
        && previous.line_cursor_chars == previous_line_len_chars
        && current.selection_start_utf16 >= previous.selection_start_utf16
        && current.selection_end_utf16 >= previous.selection_end_utf16
}

fn looks_like_blank_line_input_edit(previous: &TextSnapshot, current: &TextSnapshot) -> bool {
    previous.line_index == current.line_index
        && is_weak_signal_line(previous)
        && !current.line_text.trim().is_empty()
        && current.selection_start_utf16 >= previous.selection_start_utf16
        && current.selection_end_utf16 >= previous.selection_end_utf16
        && current.line_cursor_utf16 >= previous.line_cursor_utf16
        && current.line_cursor_chars >= previous.line_cursor_chars
}

/// Detect if the line text change looks like typing (inserting a character at cursor position).
/// This prevents IME switching when the user is actively typing.
fn looks_like_typing(previous: &TextSnapshot, current: &TextSnapshot) -> bool {
    // Must be on the same line
    if previous.line_index != current.line_index {
        return false;
    }

    let prev_chars: Vec<char> = previous.line_text.chars().collect();
    let curr_chars: Vec<char> = current.line_text.chars().collect();

    // Current line should be exactly 1 character longer than previous
    if curr_chars.len() != prev_chars.len() + 1 {
        return false;
    }

    // Cursor should have moved forward by exactly 1 character
    if current.line_cursor_chars != previous.line_cursor_chars + 1 {
        return false;
    }

    // The insertion point should be at the previous cursor position
    let insert_pos = previous.line_cursor_chars;
    if insert_pos > prev_chars.len() {
        return false;
    }

    // Check that the new line is the old line with a character inserted at cursor position
    // Before insertion: prev_chars[..insert_pos] + prev_chars[insert_pos..]
    // After insertion:  curr_chars[..insert_pos] + curr_chars[insert_pos] + curr_chars[insert_pos+1..]
    // So: curr_chars[..insert_pos] == prev_chars[..insert_pos]
    //     curr_chars[insert_pos+1..] == prev_chars[insert_pos..]
    let prefix_matches = curr_chars[..insert_pos] == prev_chars[..insert_pos];
    let suffix_matches = curr_chars[insert_pos + 1..] == prev_chars[insert_pos..];

    prefix_matches && suffix_matches
}

/// Detect if the line text change looks like IME composition (pinyin input).
/// This checks that the previous line text is a prefix of the current line text,
/// and the inserted part consists of lowercase ASCII letters or apostrophes.
/// For example: "变成中文" → "变成ces中文" (inserted "ces" before "中文")
fn looks_like_ime_composition(previous: &TextSnapshot, current: &TextSnapshot) -> bool {
    // Must be on the same line
    if previous.line_index != current.line_index {
        return false;
    }

    let prev_chars: Vec<char> = previous.line_text.chars().collect();
    let curr_chars: Vec<char> = current.line_text.chars().collect();

    let prev_len = prev_chars.len();
    let curr_len = curr_chars.len();

    // Current must be longer than previous (something was inserted)
    if curr_len <= prev_len {
        return false;
    }

    let inserted_count = curr_len - prev_len;

    // Find where the insertion happened by comparing prefix and suffix
    // The cursor position in the current line tells us where the insertion is
    let cursor = current.line_cursor_chars;

    // Insertion should be at or near the cursor
    // Check that the text before the insertion point matches
    if cursor < inserted_count {
        return false;
    }

    let insert_start = cursor - inserted_count;

    // Verify prefix matches
    if curr_chars[..insert_start] != prev_chars[..insert_start] {
        return false;
    }

    // Verify suffix matches
    if curr_chars[cursor..] != prev_chars[insert_start..] {
        return false;
    }

    // Check that all inserted characters are lowercase ASCII or apostrophe
    let inserted = &curr_chars[insert_start..cursor];
    if inserted.is_empty() {
        return false;
    }

    inserted.iter().all(|&ch| ch.is_ascii_lowercase() || ch == '\'')
}

fn is_weak_signal_line(snapshot: &TextSnapshot) -> bool {
    snapshot.line_text.trim().is_empty() || is_placeholder_only_uia_line(snapshot)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForegroundSnapshot {
    foreground_hwnd: isize,
    foreground_title: String,
    process_name: String,
    thread_id: u32,
    focus_hwnd: isize,
    focus_class: String,
    ime_mode: Option<InputMode>,
    ime_error: Option<String>,
    caret_hwnd: isize,
    caret_left: i32,
    caret_top: i32,
    caret_right: i32,
    caret_bottom: i32,
    text_snapshot: TextSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextSnapshot {
    source: &'static str,
    document_len_utf16: usize,
    selection_start_utf16: usize,
    selection_end_utf16: usize,
    line_index: usize,
    line_cursor_utf16: usize,
    line_cursor_chars: usize,
    line_text: String,
    attempts: Vec<TextReadAttempt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextReadAttempt {
    source: &'static str,
    result: TextReadResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TextReadResult {
    Unsupported(&'static str),
    Failed(&'static str),
}

impl TextReadResult {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Unsupported(reason) | Self::Failed(reason) => reason,
        }
    }
}

fn capture_foreground_snapshot() -> Result<ForegroundSnapshot, String> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return Err("GetForegroundWindow returned null".to_string());
    }

    let title = window_title(hwnd);
    let mut process_id = 0u32;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) };
    if thread_id == 0 {
        return Err("GetWindowThreadProcessId failed".to_string());
    }

    let mut gui_info = GUITHREADINFO::default();
    gui_info.cbSize = size_of::<GUITHREADINFO>() as u32;
    let ok = unsafe { GetGUIThreadInfo(thread_id, &mut gui_info) };
    if ok == 0 {
        return Err("GetGUIThreadInfo failed".to_string());
    }

    let focus_hwnd = gui_info.hwndFocus;
    let focus_class = if focus_hwnd.is_null() {
        String::new()
    } else {
        class_name(focus_hwnd)
    };
    let process_name = process_name(process_id).unwrap_or_default();

    let text_snapshot = capture_text_snapshot(focus_hwnd, &focus_class, &process_name, &title);
    let ime_target_hwnd = resolve_ime_target_hwnd(hwnd, focus_hwnd);
    let (ime_mode, ime_error) = match read_ime_mode(ime_target_hwnd) {
        Ok(mode) => (Some(mode), None),
        Err(error) => (None, Some(error)),
    };

    Ok(ForegroundSnapshot {
        foreground_hwnd: hwnd as isize,
        foreground_title: title,
        process_name,
        thread_id,
        focus_hwnd: focus_hwnd as isize,
        focus_class: focus_class.clone(),
        ime_mode,
        ime_error,
        caret_hwnd: gui_info.hwndCaret as isize,
        caret_left: gui_info.rcCaret.left,
        caret_top: gui_info.rcCaret.top,
        caret_right: gui_info.rcCaret.right,
        caret_bottom: gui_info.rcCaret.bottom,
        text_snapshot,
    })
}

fn current_ime_target_hwnd() -> Result<HWND, String> {
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

fn read_ime_mode(target_hwnd: HWND) -> Result<InputMode, String> {
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

fn write_ime_mode(target_hwnd: HWND, mode: InputMode) -> Result<(), String> {
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

/// Check if the IME is currently in composition state (user is typing pinyin, etc.)
/// Uses ImmGetCompositionStringW with GCS_COMPSTR to detect active composition.
fn is_ime_composing(target_hwnd: HWND) -> bool {
    with_ime_context(target_hwnd, |himc| {
        // GetCompositionString returns the byte count of the composition string.
        // If > 0, IME is actively composing.
        let comp_len = unsafe {
            ImmGetCompositionStringW(himc, GCS_COMPSTR, null_mut(), 0)
        };
        Ok(comp_len > 0)
    }).unwrap_or(false)
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

#[cfg(test)]
fn input_mode_from_open_status(is_open: bool) -> InputMode {
    if is_open {
        InputMode::Chinese
    } else {
        InputMode::English
    }
}

fn input_mode_from_conversion_status(conversion_mode: Dword) -> InputMode {
    if conversion_mode & IME_CMODE_NATIVE != 0 {
        InputMode::Chinese
    } else {
        InputMode::English
    }
}

fn capture_text_snapshot(
    hwnd: HWND,
    class_name: &str,
    process_name: &str,
    foreground_title: &str,
) -> TextSnapshot {
    let mut attempts = Vec::new();

    // VS Code extension via Named Pipe (highest priority for Monaco editors)
    match capture_vscode_extension_text(process_name) {
        Ok(mut snapshot) => {
            snapshot.attempts = attempts;
            return snapshot;
        }
        Err(result) => attempts.push(TextReadAttempt {
            source: "vscode_extension",
            result,
        }),
    }

    match capture_win32_edit_text(hwnd, class_name) {
        Ok(mut snapshot) => {
            snapshot.attempts = attempts;
            return snapshot;
        }
        Err(result) => attempts.push(TextReadAttempt {
            source: "win32_edit",
            result,
        }),
    }

    match capture_app_adapter_text(hwnd, class_name, process_name, foreground_title) {
        Ok(mut snapshot) => {
            snapshot.attempts = attempts;
            return snapshot;
        }
        Err(result) => attempts.push(TextReadAttempt {
            source: "app_adapter",
            result,
        }),
    }

    match capture_uia_text(hwnd) {
        Ok(mut snapshot) => {
            snapshot.attempts = attempts;
            return snapshot;
        }
        Err(result) => attempts.push(TextReadAttempt {
            source: "uia_text_pattern",
            result,
        }),
    }

    TextSnapshot {
        source: "unsupported",
        document_len_utf16: 0,
        selection_start_utf16: 0,
        selection_end_utf16: 0,
        line_index: 0,
        line_cursor_utf16: 0,
        line_cursor_chars: 0,
        line_text: String::new(),
        attempts,
    }
}

fn capture_win32_edit_text(hwnd: HWND, class_name: &str) -> Result<TextSnapshot, TextReadResult> {
    if hwnd.is_null() || !is_supported_edit_class(class_name) {
        return Err(TextReadResult::Unsupported("unsupported_control_class"));
    }

    let (selection_start_utf16, selection_end_utf16) = match edit_selection(hwnd) {
        Some(value) => value,
        None => return Err(TextReadResult::Failed("selection_read_failed")),
    };
    let line_index = match edit_line_from_char(hwnd, selection_start_utf16) {
        Some(value) => value,
        None => return Err(TextReadResult::Failed("line_from_char_failed")),
    };
    let line_start_utf16 = match edit_line_index(hwnd, line_index) {
        Some(value) => value,
        None => return Err(TextReadResult::Failed("line_index_failed")),
    };
    let line_len_utf16 = match edit_line_length(hwnd, line_start_utf16) {
        Some(value) => value,
        None => return Err(TextReadResult::Failed("line_length_failed")),
    };
    if selection_start_utf16 < line_start_utf16 {
        return Err(TextReadResult::Failed("line_range_out_of_bounds"));
    }

    let line_utf16 = match edit_get_line(hwnd, line_index, line_len_utf16) {
        Some(value) => value,
        None => return Err(TextReadResult::Failed("line_read_failed")),
    };
    let line_text = String::from_utf16_lossy(&line_utf16);
    let line_cursor_utf16 = selection_start_utf16 - line_start_utf16;
    let line_cursor_chars = match utf16_units_to_char_index(&line_utf16, line_cursor_utf16) {
        Some(value) => value,
        None => return Err(TextReadResult::Failed("cursor_utf16_to_char_failed")),
    };
    let document_len_utf16 = match window_text_utf16(hwnd) {
        Some(value) => value.len(),
        None => return Err(TextReadResult::Failed("document_length_read_failed")),
    };

    Ok(TextSnapshot {
        source: "win32_edit",
        document_len_utf16,
        selection_start_utf16,
        selection_end_utf16,
        line_index,
        line_cursor_utf16,
        line_cursor_chars,
        line_text,
        attempts: Vec::new(),
    })
}

fn capture_uia_text(hwnd: HWND) -> Result<TextSnapshot, TextReadResult> {
    if hwnd.is_null() {
        return Err(TextReadResult::Unsupported("missing_focus_hwnd"));
    }

    read_uia_text(hwnd, None)
        .map_err(|_| TextReadResult::Unsupported("uia_text_pattern_unavailable"))
}

fn capture_app_adapter_text(
    hwnd: HWND,
    class_name: &str,
    process_name: &str,
    foreground_title: &str,
) -> Result<TextSnapshot, TextReadResult> {
    let Some(adapter) = app_adapter_for_context(class_name, process_name, foreground_title) else {
        return Err(TextReadResult::Unsupported("no_app_adapter"));
    };

    read_uia_text(hwnd, Some(adapter))
        .map(|mut snapshot| {
            snapshot.source = "app_adapter";
            snapshot
        })
        .map_err(|_| TextReadResult::Unsupported("app_adapter_unavailable"))
}

fn app_adapter_for_context(
    class_name: &str,
    process_name: &str,
    _foreground_title: &str,
) -> Option<AppAdapter> {
    match (class_name, process_name) {
        ("Chrome_WidgetWin_1", process_name)
            if matches_known_chromium_editor_process(process_name) =>
        {
            Some(AppAdapter::ChromiumUia)
        }
        _ => None,
    }
}

fn matches_known_chromium_editor_process(process_name: &str) -> bool {
    process_name.eq_ignore_ascii_case("Code.exe")
        || process_name.eq_ignore_ascii_case("Cursor.exe")
        || process_name.eq_ignore_ascii_case("Obsidian.exe")
}

fn read_uia_text(
    hwnd: HWND,
    adapter: Option<AppAdapter>,
) -> Result<TextSnapshot, windows::core::Error> {
    let com_initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).is_ok() };

    let result = unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let element = focused_uia_element(&automation, hwnd)?;
        let pattern: IUIAutomationTextPattern = element.GetCurrentPatternAs(UIA_TextPatternId)?;
        let selection = pattern.GetSelection()?;
        if selection.Length()? <= 0 {
            return Err(windows::core::Error::from_win32());
        }

        let selected_range = selection.GetElement(0)?;
        let document_range = pattern.DocumentRange()?;
        uia_range_to_line_snapshot(&document_range, &selected_range, adapter)
    };

    if com_initialized {
        unsafe { CoUninitialize() };
    }

    result
}

fn focused_uia_element(
    automation: &IUIAutomation,
    hwnd: HWND,
) -> Result<IUIAutomationElement, windows::core::Error> {
    match unsafe { automation.GetFocusedElement() } {
        Ok(element) => Ok(element),
        Err(_) => unsafe { automation.ElementFromHandle(WinHwnd(hwnd)) },
    }
}

fn uia_range_to_line_snapshot(
    document_range: &IUIAutomationTextRange,
    selected_range: &IUIAutomationTextRange,
    adapter: Option<AppAdapter>,
) -> Result<TextSnapshot, windows::core::Error> {
    let line_range = unsafe { selected_range.Clone()? };
    unsafe { line_range.ExpandToEnclosingUnit(TextUnit_Line)? };

    let document_prefix = unsafe { document_range.Clone()? };
    unsafe {
        document_prefix.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            selected_range,
            TextPatternRangeEndpoint_Start,
        )?
    };

    let mut line_text =
        normalize_uia_text_for_adapter(unsafe { line_range.GetText(-1)? }.to_string(), adapter);

    let document_text_raw = unsafe { document_range.GetText(-1)? }.to_string();
    let document_text = normalize_uia_text_for_adapter(document_text_raw.clone(), adapter);
    let document_prefix_text = normalize_uia_text_for_adapter(
        unsafe { document_prefix.GetText(-1)? }.to_string(),
        adapter,
    );

    // VS Code (Chromium) UIA may fail to expand TextUnit_Line correctly, returning
    // just a single character instead of the full line. Detect this and manually
    // extract the line from the document text using newline boundaries.
    if adapter.is_some() && line_text.chars().count() <= 2 {
        let cursor_offset = document_prefix_text.chars().count();

        if let Some(extracted_line) = extract_line_from_document(&document_text, cursor_offset) {
            line_text = extracted_line;
        }
    }
    let line_index = uia_line_index(document_range, &line_range)?;
    let line_prefix = unsafe { line_range.Clone()? };
    unsafe {
        line_prefix.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            selected_range,
            TextPatternRangeEndpoint_Start,
        )?
    };
    let line_prefix_text =
        normalize_uia_text_for_adapter(unsafe { line_prefix.GetText(-1)? }.to_string(), adapter);
    let document_len_utf16 = document_text.encode_utf16().count();
    let selection_start_utf16 = document_prefix_text.encode_utf16().count();
    let selection_end_utf16 = selection_start_utf16;
    let line_prefix_utf16 = line_prefix_text.encode_utf16().count();
    let line_prefix_chars = line_prefix_text.chars().count();

    // Logical line extraction for wrapped text (non-adapter UIA only).
    // UIA TextUnit_Line returns physical visible lines; for editors with soft wrap,
    // we prefer the logical line (between \n boundaries) so the classifier sees
    // the full line context rather than a fragment.
    let (final_line_text, final_cursor_chars, final_cursor_utf16) =
        if adapter.is_none() && document_text.contains('\n') {
            let cursor_offset = document_prefix_text.chars().count();
            let chars: Vec<char> = document_text.chars().collect();
            let line_start = chars[..cursor_offset.min(chars.len())]
                .iter()
                .rposition(|&ch| ch == '\n')
                .map(|pos| pos + 1)
                .unwrap_or(0);
            let line_end = chars[cursor_offset..]
                .iter()
                .position(|&ch| ch == '\n')
                .map(|pos| cursor_offset + pos)
                .unwrap_or(chars.len());
            let logical_line: String = chars[line_start..line_end].iter().collect();
            let cursor_in_line = cursor_offset.saturating_sub(line_start);
            let cursor_in_line_utf16 = logical_line[..cursor_in_line.min(logical_line.len())]
                .chars()
                .collect::<String>()
                .encode_utf16()
                .count();
            (logical_line, cursor_in_line, cursor_in_line_utf16)
        } else {
            (line_text, line_prefix_chars, line_prefix_utf16)
        };

    Ok(TextSnapshot {
        source: "uia_text_pattern",
        document_len_utf16,
        selection_start_utf16,
        selection_end_utf16,
        line_index,
        line_cursor_utf16: final_cursor_utf16,
        line_cursor_chars: final_cursor_chars,
        line_text: final_line_text,
        attempts: Vec::new(),
    })
}

fn normalize_uia_text_for_adapter(text: String, adapter: Option<AppAdapter>) -> String {
    match adapter {
        Some(AppAdapter::ChromiumUia) => text
            .chars()
            .filter(|ch| !is_chromium_uia_ghost_char(*ch))
            .collect(),
        None => text,
    }
}

fn is_chromium_uia_ghost_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{FFFC}'
    )
}

/// Manually extract a line from document text using newline boundaries.
/// Used as a fallback when UIA TextUnit_Line expansion fails (e.g., VS Code).
fn extract_line_from_document(document_text: &str, cursor_char_offset: usize) -> Option<String> {
    let chars: Vec<char> = document_text.chars().collect();
    let total = chars.len();

    if cursor_char_offset > total {
        return None;
    }

    // Search backwards for line start (after last newline, or document start)
    let line_start = chars[..cursor_char_offset]
        .iter()
        .rposition(|&ch| ch == '\n')
        .map(|pos| pos + 1)
        .unwrap_or(0);

    // Search forwards for line end (at next newline, or document end)
    let line_end = chars[cursor_char_offset..]
        .iter()
        .position(|&ch| ch == '\n')
        .map(|pos| cursor_char_offset + pos)
        .unwrap_or(total);

    if line_start >= line_end {
        // Empty line (cursor right after a newline)
        return Some(String::new());
    }

    Some(chars[line_start..line_end].iter().collect())
}

const VSCODE_PIPE_PATH: &str = r"\\.\pipe\smart-shift-vscode";

/// Response from VS Code extension's Named Pipe
#[derive(serde::Deserialize)]
struct VsCodeLineResponse {
    line: String,
    cursor: usize,
    #[serde(rename = "lineNumber")]
    line_number: usize,
    #[allow(dead_code)]
    #[serde(rename = "totalLines")]
    total_lines: usize,
    #[serde(default)]
    composing: bool,
}

/// Try to read current line from VS Code extension via Named Pipe.
/// This is a fallback for Monaco editors (VS Code, Cursor) where UIA TextPattern is broken.
fn capture_vscode_extension_text(process_name: &str) -> Result<TextSnapshot, TextReadResult> {
    // Only try for known VS Code-based editors
    if !matches_known_chromium_editor_process(process_name) {
        return Err(TextReadResult::Unsupported("not_vscode_editor"));
    }

    let pipe_path = VSCODE_PIPE_PATH.to_string();
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let result = (|| {
            let mut pipe = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&pipe_path)
                .map_err(|_| TextReadResult::Failed("pipe_connect_failed"))?;

            // Send request
            pipe.write_all(b"GET_LINE\n")
                .map_err(|_| TextReadResult::Failed("pipe_write_failed"))?;

            let mut response = String::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = pipe.read(&mut buf).map_err(|_| TextReadResult::Failed("pipe_read_failed"))?;
                if n == 0 {
                    break;
                }
                response.push_str(&String::from_utf8_lossy(&buf[..n]));
                if response.contains('\n') {
                    break;
                }
            }

            let response = response.trim();

            // Check for error response
            if response.contains("\"error\"") {
                return Err(TextReadResult::Failed("vscode_no_editor"));
            }

            let vs_response: VsCodeLineResponse =
                serde_json::from_str(response).map_err(|_| TextReadResult::Failed("pipe_parse_failed"))?;

            // If IME is composing, return a weak signal to prevent mode switching
            if vs_response.composing {
                return Err(TextReadResult::Failed("ime_composing"));
            }

            let line_text = vs_response.line;
            let cursor_chars = vs_response.cursor;
            let line_cursor_utf16 = line_text[..line_text.chars().take(cursor_chars).collect::<String>().len()]
                .encode_utf16()
                .count();

            Ok(TextSnapshot {
                source: "vscode_extension",
                document_len_utf16: 0, // Not available from extension
                selection_start_utf16: 0,
                selection_end_utf16: 0,
                line_index: vs_response.line_number,
                line_cursor_utf16,
                line_cursor_chars: cursor_chars,
                line_text,
                attempts: Vec::new(),
            })
        })();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_millis(500)) {
        Ok(result) => result,
        Err(_) => Err(TextReadResult::Failed("pipe_timeout")),
    }
}

fn uia_line_index(
    document_range: &IUIAutomationTextRange,
    line_range: &IUIAutomationTextRange,
) -> Result<usize, windows::core::Error> {
    let walker = unsafe { document_range.Clone()? };
    unsafe {
        walker.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            document_range,
            TextPatternRangeEndpoint_Start,
        )?;
        walker.ExpandToEnclosingUnit(TextUnit_Line)?;
    }

    let mut line_index = 0usize;
    loop {
        let comparison = unsafe {
            walker.CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                line_range,
                TextPatternRangeEndpoint_Start,
            )?
        };
        if comparison >= 0 {
            return Ok(line_index);
        }

        let moved = unsafe { walker.Move(TextUnit_Line, 1)? };
        if moved == 0 {
            return Ok(line_index);
        }
        line_index += moved as usize;
    }
}

fn print_snapshot(
    snapshot: &ForegroundSnapshot,
    debug: bool,
    app_handle: &Option<tauri::AppHandle>,
    event_logger: Option<&EventLogger>,
) {
    println!();
    println!("{}smart-shift event{}", STYLE_BOLD, STYLE_RESET);

    if debug {
        println!(
            "{}Window{}   0x{:X}  thread={}  process={}  title=\"{}\"",
            COLOR_DIM,
            STYLE_RESET,
            snapshot.foreground_hwnd,
            snapshot.thread_id,
            if snapshot.process_name.is_empty() {
                "unknown"
            } else {
                &snapshot.process_name
            },
            snapshot.foreground_title
        );
        println!(
            "{}Focus{}    0x{:X}  class={}",
            COLOR_DIM, STYLE_RESET, snapshot.focus_hwnd, snapshot.focus_class
        );
        if let Some(error) = &snapshot.ime_error {
            println!("{}IME read error{} {error}", COLOR_DIM, STYLE_RESET);
        }
        println!(
            "{}Caret{}    hwnd=0x{:X}  rect=({}, {}, {}, {})",
            COLOR_DIM,
            STYLE_RESET,
            snapshot.caret_hwnd,
            snapshot.caret_left,
            snapshot.caret_top,
            snapshot.caret_right,
            snapshot.caret_bottom
        );
    }

    if snapshot.text_snapshot.source != "unsupported" {
        let edit = &snapshot.text_snapshot;
        println!(
            "{}Line{}     \"{}\"",
            COLOR_CYAN, STYLE_RESET, edit.line_text
        );

        if debug {
            println!(
                "{}Text{}     source={}  doc_len_utf16={}  selection=({}, {})",
                COLOR_DIM,
                STYLE_RESET,
                edit.source,
                edit.document_len_utf16,
                edit.selection_start_utf16,
                edit.selection_end_utf16,
            );
            println!(
                "{}         line={}  cursor_utf16={}  cursor_chars={}{}",
                COLOR_DIM,
                edit.line_index,
                edit.line_cursor_utf16,
                edit.line_cursor_chars,
                STYLE_RESET
            );
        }

        print_snapshot_decision(snapshot, debug, app_handle, event_logger);
    } else {
        println!("{}Line{}     unsupported", COLOR_YELLOW, STYLE_RESET);
        println!(
            "{}Switch{}   skipped  reason=text_unsupported",
            COLOR_YELLOW, STYLE_RESET
        );
        if debug {
            for attempt in &snapshot.text_snapshot.attempts {
                println!(
                    "{}         attempt source={} result={}{}",
                    COLOR_DIM,
                    attempt.source,
                    attempt.result.as_str(),
                    STYLE_RESET
                );
            }
        }

        emit_watcher_event(
            app_handle,
            event_logger,
            WatcherEvent {
                line_text: String::new(),
                source: "unsupported".to_string(),
                current_mode: snapshot.ime_mode.map(|m| format!("{}", m)),
                target_mode: None,
                switched: false,
                preserved: false,
                reason: "text_unsupported".to_string(),
                error: None,
                debug,
                window_title: if debug { Some(snapshot.foreground_title.clone()) } else { None },
                process_name: if debug { Some(snapshot.process_name.clone()) } else { None },
                focus_class: if debug { Some(snapshot.focus_class.clone()) } else { None },
                ime_error: if debug { snapshot.ime_error.clone() } else { None },
                caret: if debug { Some(format!("hwnd=0x{:X} rect=({}, {}, {}, {})", snapshot.caret_hwnd, snapshot.caret_left, snapshot.caret_top, snapshot.caret_right, snapshot.caret_bottom)) } else { None },
                doc_len: None,
                selection_start: None,
                selection_end: None,
                line_number: None,
                cursor_utf16: None,
                cursor_chars: None,
            },
        );
    }
}

fn print_snapshot_decision(
    snapshot: &ForegroundSnapshot,
    debug: bool,
    app_handle: &Option<tauri::AppHandle>,
    event_logger: Option<&EventLogger>,
) {
    let edit = &snapshot.text_snapshot;
    let current_mode = snapshot.ime_mode;
    match classify_snapshot(edit) {
        Ok(decision) => {
            if should_preserve_current_mode(edit, &decision) {
                println!(
                    "{}IME{}      current={}  target={}",
                    COLOR_CYAN,
                    STYLE_RESET,
                    format_mode(current_mode),
                    format_mode(current_mode)
                );
                if debug {
                    println!(
                        "{}Decision{} skipped  reason=weak_signal_preserve_mode  classifier_reason={}",
                        COLOR_DIM, STYLE_RESET, decision.reason
                    );
                }
                println!(
                    "{}Switch{}   skipped  reason=weak_signal_preserve_mode",
                    COLOR_YELLOW, STYLE_RESET
                );

                emit_watcher_event(
                    app_handle,
                    event_logger,
                    WatcherEvent {
                        line_text: edit.line_text.clone(),
                        source: edit.source.to_string(),
                        current_mode: current_mode.map(|m| format!("{}", m)),
                        target_mode: current_mode.map(|m| format!("{}", m)),
                        switched: false,
                        preserved: true,
                        reason: format!("{}", decision.reason),
                        error: None,
                        debug,
                        window_title: if debug { Some(snapshot.foreground_title.clone()) } else { None },
                        process_name: if debug { Some(snapshot.process_name.clone()) } else { None },
                        focus_class: if debug { Some(snapshot.focus_class.clone()) } else { None },
                        ime_error: if debug { snapshot.ime_error.clone() } else { None },
                        caret: if debug { Some(format!("hwnd=0x{:X} rect=({}, {}, {}, {})", snapshot.caret_hwnd, snapshot.caret_left, snapshot.caret_top, snapshot.caret_right, snapshot.caret_bottom)) } else { None },
                        doc_len: if debug { Some(edit.document_len_utf16) } else { None },
                        selection_start: if debug { Some(edit.selection_start_utf16) } else { None },
                        selection_end: if debug { Some(edit.selection_end_utf16) } else { None },
                        line_number: if debug { Some(edit.line_index) } else { None },
                        cursor_utf16: if debug { Some(edit.line_cursor_utf16) } else { None },
                        cursor_chars: if debug { Some(edit.line_cursor_chars) } else { None },
                    },
                );
                return;
            }

            println!(
                "{}IME{}      current={}  target={}",
                COLOR_CYAN,
                STYLE_RESET,
                format_mode(current_mode),
                format_mode(Some(decision.mode))
            );
            if debug {
                println!(
                    "{}Decision{} reason={}",
                    COLOR_DIM, STYLE_RESET, decision.reason
                );
            }
            let switch_result = print_watcher_switch(decision.mode);
            let (switched, error) = match switch_result {
                Ok(WatcherSwitchOutcome::Applied) => (true, None),
                Ok(WatcherSwitchOutcome::SkippedAlreadyMatched)
                | Ok(WatcherSwitchOutcome::SkippedUnknownCurrentMode) => (false, None),
                Err(error) => (false, Some(error)),
            };

            emit_watcher_event(
                app_handle,
                event_logger,
                WatcherEvent {
                    line_text: edit.line_text.clone(),
                    source: edit.source.to_string(),
                    current_mode: current_mode.map(|m| format!("{}", m)),
                    target_mode: Some(format!("{}", decision.mode)),
                    switched,
                    preserved: false,
                    reason: format!("{}", decision.reason),
                    error,
                    debug,
                    window_title: if debug { Some(snapshot.foreground_title.clone()) } else { None },
                    process_name: if debug { Some(snapshot.process_name.clone()) } else { None },
                    focus_class: if debug { Some(snapshot.focus_class.clone()) } else { None },
                    ime_error: if debug { snapshot.ime_error.clone() } else { None },
                    caret: if debug { Some(format!("hwnd=0x{:X} rect=({}, {}, {}, {})", snapshot.caret_hwnd, snapshot.caret_left, snapshot.caret_top, snapshot.caret_right, snapshot.caret_bottom)) } else { None },
                    doc_len: if debug { Some(edit.document_len_utf16) } else { None },
                    selection_start: if debug { Some(edit.selection_start_utf16) } else { None },
                    selection_end: if debug { Some(edit.selection_end_utf16) } else { None },
                    line_number: if debug { Some(edit.line_index) } else { None },
                    cursor_utf16: if debug { Some(edit.line_cursor_utf16) } else { None },
                    cursor_chars: if debug { Some(edit.line_cursor_chars) } else { None },
                },
            );
        }
        Err((cursor, text_len)) => {
            println!(
                "{}IME{}      current={}  target=unknown",
                COLOR_CYAN,
                STYLE_RESET,
                format_mode(current_mode)
            );
            if debug {
                println!(
                    "{}Decision{} unavailable  cursor={cursor}  text_len={text_len}",
                    COLOR_DIM, STYLE_RESET
                );
            }
            println!(
                "{}Switch{}   skipped  reason=classification_unavailable",
                COLOR_YELLOW, STYLE_RESET
            );

            emit_watcher_event(
                app_handle,
                event_logger,
                WatcherEvent {
                    line_text: edit.line_text.clone(),
                    source: edit.source.to_string(),
                    current_mode: current_mode.map(|m| format!("{}", m)),
                    target_mode: None,
                    switched: false,
                    preserved: false,
                    reason: "classification_unavailable".to_string(),
                    error: Some(format!("cursor={cursor} text_len={text_len}")),
                    debug,
                    window_title: if debug { Some(snapshot.foreground_title.clone()) } else { None },
                    process_name: if debug { Some(snapshot.process_name.clone()) } else { None },
                    focus_class: if debug { Some(snapshot.focus_class.clone()) } else { None },
                    ime_error: if debug { snapshot.ime_error.clone() } else { None },
                    caret: if debug { Some(format!("hwnd=0x{:X} rect=({}, {}, {}, {})", snapshot.caret_hwnd, snapshot.caret_left, snapshot.caret_top, snapshot.caret_right, snapshot.caret_bottom)) } else { None },
                    doc_len: if debug { Some(edit.document_len_utf16) } else { None },
                    selection_start: if debug { Some(edit.selection_start_utf16) } else { None },
                    selection_end: if debug { Some(edit.selection_end_utf16) } else { None },
                    line_number: if debug { Some(edit.line_index) } else { None },
                    cursor_utf16: if debug { Some(edit.line_cursor_utf16) } else { None },
                    cursor_chars: if debug { Some(edit.line_cursor_chars) } else { None },
                },
            );
        }
    }
}

fn classify_snapshot(snapshot: &TextSnapshot) -> Result<Decision, (usize, usize)> {
    let context = LineContext::new(snapshot.line_text.clone(), snapshot.line_cursor_chars)?;
    Ok(classify(&context))
}

fn should_preserve_current_mode(snapshot: &TextSnapshot, decision: &Decision) -> bool {
    decision.reason == DecisionReason::DefaultChinese && is_weak_signal_line(snapshot)
}

fn is_placeholder_only_uia_line(snapshot: &TextSnapshot) -> bool {
    snapshot.source == "uia_text_pattern"
        && !snapshot.line_text.trim().is_empty()
        && snapshot
            .line_text
            .chars()
            .all(|ch| ch.is_whitespace() || !ch.is_alphanumeric())
}

fn emit_watcher_event(
    app_handle: &Option<tauri::AppHandle>,
    event_logger: Option<&EventLogger>,
    event: WatcherEvent,
) {
    if let Some(app_handle) = app_handle {
        let _ = app_handle.emit("watcher-event", event.clone());
    }
    if let Some(event_logger) = event_logger {
        let _ = event_logger.append_event(&event);
    }
}

fn print_watcher_switch(target_mode: InputMode) -> Result<WatcherSwitchOutcome, String> {
    let controller = WindowsImeController::new();
    let current_mode = controller.current_mode().ok();

    match maybe_switch_watcher_mode(&controller, current_mode, target_mode) {
        Ok(outcome) => {
            let switch_summary = match outcome {
                WatcherSwitchOutcome::Applied => "applied  reason=mode_changed",
                WatcherSwitchOutcome::SkippedAlreadyMatched => "skipped  reason=already_matched",
                WatcherSwitchOutcome::SkippedUnknownCurrentMode => {
                    "skipped  reason=current_mode_unknown"
                }
            };

            match outcome {
                WatcherSwitchOutcome::Applied
                | WatcherSwitchOutcome::SkippedAlreadyMatched
                | WatcherSwitchOutcome::SkippedUnknownCurrentMode => {
                    match controller.current_mode() {
                        Ok(mode) => println!(
                            "{}Switch{}   {switch_summary}  after={}",
                            switch_color(outcome),
                            STYLE_RESET,
                            format_mode(Some(mode))
                        ),
                        Err(error) => {
                            println!(
                                "{}Switch{}   {switch_summary}  after=unknown",
                                COLOR_YELLOW, STYLE_RESET
                            );
                            println!("{}         read_error={error}{}", COLOR_DIM, STYLE_RESET);
                        }
                    }
                }
            };

            Ok(outcome)
        }
        Err(error) => {
            match controller.current_mode() {
                Ok(mode) => println!(
                    "{}Switch{}   failed  reason=apply_failed  after={}",
                    COLOR_RED,
                    STYLE_RESET,
                    format_mode(Some(mode))
                ),
                Err(error) => {
                    println!(
                        "{}Switch{}   failed  reason=apply_failed  after=unknown",
                        COLOR_RED, STYLE_RESET
                    );
                    println!("{}         read_error={error}{}", COLOR_DIM, STYLE_RESET);
                }
            }
            println!("{}         switch_error={error}{}", COLOR_DIM, STYLE_RESET);

            Err(error)
        }
    }
}

fn format_mode(mode: Option<InputMode>) -> String {
    match mode {
        Some(InputMode::Chinese) => format!("{COLOR_GREEN}chinese{STYLE_RESET}"),
        Some(InputMode::English) => format!("{COLOR_CYAN}english{STYLE_RESET}"),
        None => format!("{COLOR_YELLOW}unknown{STYLE_RESET}"),
    }
}

fn switch_color(outcome: WatcherSwitchOutcome) -> &'static str {
    match outcome {
        WatcherSwitchOutcome::Applied => COLOR_GREEN,
        WatcherSwitchOutcome::SkippedAlreadyMatched
        | WatcherSwitchOutcome::SkippedUnknownCurrentMode => COLOR_YELLOW,
    }
}

#[cfg(test)]
fn tray_command_id(wparam: Wparam) -> usize {
    wparam & 0xFFFF
}

#[cfg(test)]
fn tray_callback_event(lparam: Lparam) -> UINT {
    (lparam as usize & 0xFFFF) as UINT
}

fn maybe_switch_watcher_mode(
    controller: &WindowsImeController,
    current_mode: Option<InputMode>,
    target_mode: InputMode,
) -> Result<WatcherSwitchOutcome, String> {
    match current_mode {
        Some(mode) if mode == target_mode => Ok(WatcherSwitchOutcome::SkippedAlreadyMatched),
        Some(_) => {
            controller.switch_to(target_mode)?;
            Ok(WatcherSwitchOutcome::Applied)
        }
        None => Ok(WatcherSwitchOutcome::SkippedUnknownCurrentMode),
    }
}

fn window_title(hwnd: HWND) -> String {
    let text = window_text_utf16(hwnd).unwrap_or_default();
    String::from_utf16_lossy(&text)
}

fn process_name(process_id: u32) -> Result<String, String> {
    if process_id == 0 {
        return Err("missing_process_id".to_string());
    }

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(format!("OpenProcess failed for pid={process_id}"));
    }

    let mut buffer = vec![0u16; 260];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size) };
    let close_ok = unsafe { CloseHandle(process) };
    if close_ok == 0 {
        return Err(format!("CloseHandle failed for pid={process_id}"));
    }
    if ok == 0 {
        return Err(format!(
            "QueryFullProcessImageNameW failed for pid={process_id}"
        ));
    }

    buffer.truncate(size as usize);
    let path = String::from_utf16_lossy(&buffer);
    Ok(path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_string())
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = vec![0u16; 256];
    let written = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    if written <= 0 {
        return String::new();
    }

    String::from_utf16_lossy(&buffer[..written as usize])
}

fn window_text_utf16(hwnd: HWND) -> Option<Vec<u16>> {
    let message_length = unsafe { SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0) };
    let length = if message_length >= 0 {
        message_length as i32
    } else {
        unsafe { GetWindowTextLengthW(hwnd) }
    };
    if length < 0 {
        return None;
    }

    let mut buffer = vec![0u16; length as usize + 1];
    let written = unsafe {
        SendMessageW(
            hwnd,
            WM_GETTEXT,
            buffer.len(),
            buffer.as_mut_ptr() as LPARAM,
        )
    };
    if written < 0 {
        return None;
    }

    buffer.truncate(written as usize);
    Some(buffer)
}

fn edit_selection(hwnd: HWND) -> Option<(usize, usize)> {
    let packed = unsafe { SendMessageW(hwnd, EM_GETSEL, 0, 0) };
    if packed < 0 {
        return None;
    }

    let packed = packed as u32;
    let start = (packed & 0xFFFF) as usize;
    let end = ((packed >> 16) & 0xFFFF) as usize;
    Some((start, end))
}

fn edit_line_from_char(hwnd: HWND, char_index_utf16: usize) -> Option<usize> {
    let result = unsafe { SendMessageW(hwnd, EM_LINEFROMCHAR, char_index_utf16, 0) };
    if result < 0 {
        return None;
    }
    Some(result as usize)
}

fn edit_line_index(hwnd: HWND, line_index: usize) -> Option<usize> {
    let result = unsafe { SendMessageW(hwnd, EM_LINEINDEX, line_index, 0) };
    if result < 0 {
        return None;
    }
    Some(result as usize)
}

fn edit_line_length(hwnd: HWND, char_index_utf16: usize) -> Option<usize> {
    let result = unsafe { SendMessageW(hwnd, EM_LINELENGTH, char_index_utf16, 0) };
    if result < 0 {
        return None;
    }
    Some(result as usize)
}

fn edit_get_line(hwnd: HWND, line_index: usize, line_len_utf16: usize) -> Option<Vec<u16>> {
    let mut buffer = vec![0u16; line_len_utf16.saturating_add(1)];
    let max_len = u16::try_from(buffer.len().saturating_sub(1)).ok()?;
    buffer[0] = max_len;

    let copied =
        unsafe { SendMessageW(hwnd, EM_GETLINE, line_index, buffer.as_mut_ptr() as LPARAM) };
    if copied < 0 {
        return None;
    }

    let copied = copied as usize;
    if copied > buffer.len().saturating_sub(1) {
        return None;
    }

    Some(buffer[1..1 + copied].to_vec())
}

fn utf16_units_to_char_index(utf16_slice: &[u16], utf16_units: usize) -> Option<usize> {
    if utf16_units > utf16_slice.len() {
        return None;
    }

    let prefix = String::from_utf16_lossy(&utf16_slice[..utf16_units]);
    Some(prefix.chars().count())
}

fn is_supported_edit_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "Edit" | "RichEdit20W" | "RichEdit50W" | "RICHEDIT50W" | "RichEditD2DPT"
    )
}

type Bool = i32;
type Dword = u32;
type Hwnd = *mut c_void;
type HWND = Hwnd;
type Handle = *mut c_void;
type HANDLE = Handle;
type Lparam = isize;
type Wparam = usize;
type Lresult = isize;
type LPARAM = Lparam;
type WPARAM = Wparam;
type LRESULT = Lresult;
type UINT = u32;

const WM_GETTEXT: UINT = 0x000D;
const WM_GETTEXTLENGTH: UINT = 0x000E;
const WM_IME_CONTROL: UINT = 0x0283;
#[cfg(test)]
const WM_LBUTTONUP: UINT = 0x0202;
#[cfg(test)]
const WM_RBUTTONUP: UINT = 0x0205;
const EM_GETSEL: UINT = 0x00B0;
const EM_GETLINE: UINT = 0x00C4;
const EM_LINEFROMCHAR: UINT = 0x00C9;
const EM_LINEINDEX: UINT = 0x00BB;
const EM_LINELENGTH: UINT = 0x00C1;
const IMC_GETCONVERSIONMODE: WPARAM = 0x0001;
const IMC_SETCONVERSIONMODE: WPARAM = 0x0002;
const IMC_GETOPENSTATUS: WPARAM = 0x0005;
const IMC_SETOPENSTATUS: WPARAM = 0x0006;
const IME_CMODE_NATIVE: Dword = 0x0001;
const PROCESS_QUERY_LIMITED_INFORMATION: Dword = 0x1000;

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

#[link(name = "user32")]
unsafe extern "system" {
    fn GetForegroundWindow() -> HWND;
    fn GetWindowTextLengthW(hWnd: HWND) -> i32;
    fn GetWindowThreadProcessId(hWnd: HWND, lpdwProcessId: *mut Dword) -> Dword;
    fn GetGUIThreadInfo(idThread: Dword, lpgui: *mut GUITHREADINFO) -> Bool;
    fn GetClassNameW(hWnd: HWND, lpClassName: *mut u16, nMaxCount: i32) -> i32;
    fn SendMessageW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(lpMutexAttributes: *mut c_void, bInitialOwner: Bool, lpName: PCWSTR) -> HANDLE;
    fn CloseHandle(hObject: HANDLE) -> Bool;
    fn OpenProcess(dwDesiredAccess: Dword, bInheritHandle: Bool, dwProcessId: Dword) -> HANDLE;
    fn QueryFullProcessImageNameW(
        hProcess: HANDLE,
        dwFlags: Dword,
        lpExeName: *mut u16,
        lpdwSize: *mut Dword,
    ) -> Bool;
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
    fn ImmGetCompositionStringW(
        hIMC: *mut c_void,
        dwIndex: Dword,
        lpBuf: *mut c_void,
        dwBufLen: Dword,
    ) -> i32;
}

const GCS_COMPSTR: Dword = 0x0008;

#[cfg(test)]
mod tests {
    use super::{
        app_adapter_for_context, extract_line_from_document, ime_conversion_status_for_mode,
        ime_open_status_for_mode, input_mode_from_conversion_status, input_mode_from_open_status,
        maybe_switch_watcher_mode, normalize_uia_text_for_adapter, resolve_ime_target_hwnd,
        utf16_units_to_char_index, AppAdapter, ForegroundSnapshot, TextReadAttempt, TextSnapshot,
        TrayRuntimeControl, WatcherSwitchOutcome, Wparam, IME_CMODE_NATIVE, WM_LBUTTONUP,
        WM_RBUTTONUP,
    };
    use crate::ime::InputMode;
    use std::ptr::null_mut;

    fn snapshot(line_text: &str) -> ForegroundSnapshot {
        ForegroundSnapshot {
            foreground_hwnd: 1,
            foreground_title: "title".to_string(),
            process_name: "smart-shift-tests.exe".to_string(),
            thread_id: 1,
            focus_hwnd: 2,
            focus_class: "Edit".to_string(),
            ime_mode: Some(InputMode::English),
            ime_error: None,
            caret_hwnd: 3,
            caret_left: 10,
            caret_top: 20,
            caret_right: 11,
            caret_bottom: 30,
            text_snapshot: TextSnapshot {
                source: "win32_edit",
                document_len_utf16: line_text.encode_utf16().count(),
                selection_start_utf16: 0,
                selection_end_utf16: 0,
                line_index: 0,
                line_cursor_utf16: 0,
                line_cursor_chars: 0,
                line_text: line_text.to_string(),
                attempts: Vec::<TextReadAttempt>::new(),
            },
        }
    }

    fn uia_snapshot(line_text: &str) -> ForegroundSnapshot {
        let mut snapshot = snapshot(line_text);
        snapshot.text_snapshot.source = "uia_text_pattern";
        snapshot
    }

    #[test]
    fn selects_chromium_adapter_for_chrome_widget() {
        assert_eq!(
            app_adapter_for_context("Chrome_WidgetWin_1", "Obsidian.exe", "note.md - Obsidian"),
            Some(AppAdapter::ChromiumUia)
        );
    }

    #[test]
    fn does_not_select_adapter_for_unknown_chrome_widget_process() {
        assert_eq!(
            app_adapter_for_context(
                "Chrome_WidgetWin_1",
                "chrome.exe",
                "Example - Google Chrome"
            ),
            None
        );
    }

    #[test]
    fn does_not_select_adapter_for_plain_edit_control() {
        assert_eq!(
            app_adapter_for_context("Edit", "notepad.exe", "Untitled - Notepad"),
            None
        );
    }

    #[test]
    fn chromium_adapter_strips_invisible_uia_ghost_chars() {
        assert_eq!(
            normalize_uia_text_for_adapter(
                "\u{FFFC}he\u{200B}l\u{FEFF}lo\u{2060}".to_string(),
                Some(AppAdapter::ChromiumUia)
            ),
            "hello"
        );
    }

    #[test]
    fn chromium_adapter_preserves_visible_markdown_text() {
        assert_eq!(
            normalize_uia_text_for_adapter(
                "## heading []".to_string(),
                Some(AppAdapter::ChromiumUia)
            ),
            "## heading []"
        );
    }

    #[test]
    fn utf16_offsets_match_ascii_char_count() {
        let data: Vec<u16> = "hello".encode_utf16().collect();
        assert_eq!(utf16_units_to_char_index(&data, 3), Some(3));
    }

    #[test]
    fn utf16_offsets_handle_cjk_and_surrogates() {
        let text = "中a🙂";
        let data: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(utf16_units_to_char_index(&data, 1), Some(1));
        assert_eq!(utf16_units_to_char_index(&data, 2), Some(2));
        assert_eq!(utf16_units_to_char_index(&data, 4), Some(3));
    }

    #[test]
    fn ignores_plain_text_changes() {
        let previous = snapshot("abc");
        let mut current = snapshot("abcd");
        current.text_snapshot.selection_start_utf16 = 1;
        current.text_snapshot.selection_end_utf16 = 1;
        current.text_snapshot.line_cursor_utf16 = 1;
        current.text_snapshot.line_cursor_chars = 1;
        current.caret_left = 20;
        current.caret_right = 21;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::SuppressedTextEdit
        );
    }

    #[test]
    fn emits_for_cursor_move_without_text_change() {
        let previous = snapshot("abc");
        let mut current = snapshot("abc");
        current.text_snapshot.selection_start_utf16 = 1;
        current.text_snapshot.selection_end_utf16 = 1;
        current.text_snapshot.line_cursor_utf16 = 1;
        current.text_snapshot.line_cursor_chars = 1;
        current.caret_left = 20;
        current.caret_right = 21;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Emit
        );
    }

    #[test]
    fn emits_for_focus_change() {
        let previous = snapshot("abc");
        let mut current = snapshot("xyz");
        current.focus_hwnd = 99;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Emit
        );
    }

    #[test]
    fn ignores_ime_mode_change_without_relocation() {
        let previous = snapshot("abc");
        let mut current = snapshot("abc");
        current.ime_mode = Some(InputMode::Chinese);

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Ignore
        );
    }

    #[test]
    fn ignores_ime_error_change_without_relocation() {
        let previous = snapshot("abc");
        let mut current = snapshot("abc");
        current.ime_mode = None;
        current.ime_error = Some("ImmGetContext returned null".to_string());

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Ignore
        );
    }

    #[test]
    fn emits_cursor_move_after_suppressed_text_change_when_baseline_updates() {
        let previous = snapshot("abc");
        let mut typed = snapshot("abcd");
        typed.text_snapshot.selection_start_utf16 = 1;
        typed.text_snapshot.selection_end_utf16 = 1;
        typed.text_snapshot.line_cursor_utf16 = 1;
        typed.text_snapshot.line_cursor_chars = 1;
        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &typed, false),
            super::SnapshotTransition::SuppressedTextEdit
        );

        let mut moved = typed.clone();
        moved.text_snapshot.selection_start_utf16 = 2;
        moved.text_snapshot.selection_end_utf16 = 2;
        moved.text_snapshot.line_cursor_utf16 = 2;
        moved.text_snapshot.line_cursor_chars = 2;
        moved.caret_left = 30;
        moved.caret_right = 31;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&typed), &moved, false),
            super::SnapshotTransition::Emit
        );
    }

    #[test]
    fn emits_when_cursor_moves_to_another_line_with_different_text() {
        let previous = snapshot("first line");
        let mut current = snapshot("second line");
        current.text_snapshot.document_len_utf16 = previous.text_snapshot.document_len_utf16;
        current.text_snapshot.line_index = 1;
        current.text_snapshot.selection_start_utf16 = 3;
        current.text_snapshot.selection_end_utf16 = 3;
        current.text_snapshot.line_cursor_utf16 = 3;
        current.text_snapshot.line_cursor_chars = 3;
        current.caret_top = 40;
        current.caret_bottom = 50;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Emit
        );
    }

    #[test]
    fn emits_for_wrapped_line_move_when_document_length_is_unchanged() {
        let previous = snapshot("wrapped line segment a");
        let mut current = snapshot("wrapped line segment b");
        current.text_snapshot.selection_start_utf16 = 124;
        current.text_snapshot.selection_end_utf16 = 124;
        current.text_snapshot.line_cursor_utf16 = 54;
        current.text_snapshot.line_cursor_chars = 54;
        current.text_snapshot.document_len_utf16 = previous.text_snapshot.document_len_utf16;

        let mut previous = previous;
        previous.text_snapshot.selection_start_utf16 = 122;
        previous.text_snapshot.selection_end_utf16 = 122;
        previous.text_snapshot.line_cursor_utf16 = 52;
        previous.text_snapshot.line_cursor_chars = 52;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Emit
        );
    }

    #[test]
    fn still_ignores_same_line_text_edits() {
        let previous = snapshot("abc");
        let mut current = snapshot("abcd");
        current.text_snapshot.selection_start_utf16 = 4;
        current.text_snapshot.selection_end_utf16 = 4;
        current.text_snapshot.line_cursor_utf16 = 4;
        current.text_snapshot.line_cursor_chars = 4;
        current.caret_left = 40;
        current.caret_right = 41;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Ignore
        );
    }

    #[test]
    fn ignores_same_line_text_change_when_document_length_is_unchanged() {
        let previous = uia_snapshot("");
        let mut current = uia_snapshot("a");
        current.text_snapshot.document_len_utf16 = previous.text_snapshot.document_len_utf16;
        current.text_snapshot.selection_start_utf16 = 1;
        current.text_snapshot.selection_end_utf16 = 1;
        current.text_snapshot.line_cursor_utf16 = 1;
        current.text_snapshot.line_cursor_chars = 1;
        current.caret_left = 20;
        current.caret_right = 21;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Ignore
        );
    }

    #[test]
    fn emits_for_line_change_even_when_document_length_changes() {
        let previous = snapshot("first line");
        let mut current = snapshot("second line");
        current.text_snapshot.document_len_utf16 = previous.text_snapshot.document_len_utf16 + 1;
        current.text_snapshot.line_index = 1;
        current.text_snapshot.selection_start_utf16 = 11;
        current.text_snapshot.selection_end_utf16 = 11;
        current.text_snapshot.line_cursor_utf16 = 0;
        current.text_snapshot.line_cursor_chars = 0;
        current.caret_top = 40;
        current.caret_bottom = 50;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Emit
        );
    }

    #[test]
    fn ignores_newline_text_edit_even_when_line_changes() {
        let mut previous = snapshot("first line");
        previous.text_snapshot.selection_start_utf16 = previous.text_snapshot.document_len_utf16;
        previous.text_snapshot.selection_end_utf16 = previous.text_snapshot.document_len_utf16;
        previous.text_snapshot.line_cursor_utf16 = previous.text_snapshot.document_len_utf16;
        previous.text_snapshot.line_cursor_chars = previous.text_snapshot.line_text.chars().count();

        let mut current = snapshot("");
        current.text_snapshot.document_len_utf16 = previous.text_snapshot.document_len_utf16 + 2;
        current.text_snapshot.line_index = 1;
        current.text_snapshot.selection_start_utf16 = previous.text_snapshot.document_len_utf16 + 2;
        current.text_snapshot.selection_end_utf16 = previous.text_snapshot.document_len_utf16 + 2;
        current.text_snapshot.line_cursor_utf16 = 0;
        current.text_snapshot.line_cursor_chars = 0;
        current.caret_top = 40;
        current.caret_bottom = 50;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::SuppressedTextEdit
        );
    }

    #[test]
    fn emits_uia_line_move_when_document_length_changes() {
        let previous = uia_snapshot("  ## PasteDrop");
        let mut current = uia_snapshot("publish to juejin, linux do, hello github, ruan");
        current.text_snapshot.document_len_utf16 = previous.text_snapshot.document_len_utf16 - 3;
        current.text_snapshot.line_index = 10;
        current.text_snapshot.selection_start_utf16 = 93;
        current.text_snapshot.selection_end_utf16 = 93;
        current.text_snapshot.line_cursor_utf16 = 6;
        current.text_snapshot.line_cursor_chars = 6;

        let mut previous = previous;
        previous.text_snapshot.line_index = 9;
        previous.text_snapshot.selection_start_utf16 = 84;
        previous.text_snapshot.selection_end_utf16 = 84;
        previous.text_snapshot.line_cursor_utf16 = 9;
        previous.text_snapshot.line_cursor_chars = 9;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Emit
        );
    }

    #[test]
    fn still_ignores_uia_same_line_text_edits() {
        let previous = uia_snapshot("abc");
        let mut current = uia_snapshot("abcd");
        current.text_snapshot.selection_start_utf16 = 4;
        current.text_snapshot.selection_end_utf16 = 4;
        current.text_snapshot.line_cursor_utf16 = 4;
        current.text_snapshot.line_cursor_chars = 4;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Ignore
        );
    }

    #[test]
    fn ignores_followup_cursor_move_after_newline_text_edit() {
        let mut previous = snapshot("让我测试看看");
        previous.text_snapshot.selection_start_utf16 = previous.text_snapshot.document_len_utf16;
        previous.text_snapshot.selection_end_utf16 = previous.text_snapshot.document_len_utf16;
        previous.text_snapshot.line_cursor_utf16 = previous.text_snapshot.document_len_utf16;
        previous.text_snapshot.line_cursor_chars = previous.text_snapshot.line_text.chars().count();

        let mut intermediate = previous.clone();
        intermediate.text_snapshot.document_len_utf16 += 2;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &intermediate, false),
            super::SnapshotTransition::Ignore
        );

        let mut current = snapshot("");
        current.text_snapshot.document_len_utf16 = intermediate.text_snapshot.document_len_utf16;
        current.text_snapshot.line_index = 1;
        current.text_snapshot.selection_start_utf16 =
            previous.text_snapshot.selection_start_utf16 + 2;
        current.text_snapshot.selection_end_utf16 = previous.text_snapshot.selection_end_utf16 + 2;
        current.text_snapshot.line_cursor_utf16 = 0;
        current.text_snapshot.line_cursor_chars = 0;
        current.caret_top = 40;
        current.caret_bottom = 50;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&intermediate), &current, true),
            super::SnapshotTransition::Ignore
        );
    }

    #[test]
    fn emits_for_cursor_move_when_document_len_fluctuates_from_uia_noise() {
        // Simulates Chromium UIA ghost-char noise: line text is unchanged, but
        // document_len_utf16 fluctuates slightly between polls.
        let previous = uia_snapshot("hello world");
        let mut current = uia_snapshot("hello world");
        current.text_snapshot.document_len_utf16 = previous.text_snapshot.document_len_utf16 + 1;
        current.text_snapshot.selection_start_utf16 = 3;
        current.text_snapshot.selection_end_utf16 = 3;
        current.text_snapshot.line_cursor_utf16 = 3;
        current.text_snapshot.line_cursor_chars = 3;
        current.caret_left = 30;
        current.caret_right = 31;

        assert_eq!(
            super::classify_snapshot_transition_with_state(Some(&previous), &current, false),
            super::SnapshotTransition::Emit
        );
    }

    #[test]
    fn classifies_supported_snapshot_line() {
        let mut snapshot = snapshot("hello world");
        snapshot.text_snapshot.line_cursor_chars = 1;

        let decision = super::classify_snapshot(&snapshot.text_snapshot).unwrap();

        assert_eq!(decision.mode, InputMode::English);
    }

    #[test]
    fn reports_unavailable_classification_for_invalid_cursor() {
        let mut snapshot = snapshot("abc");
        snapshot.text_snapshot.line_cursor_chars = 4;

        assert_eq!(
            super::classify_snapshot(&snapshot.text_snapshot),
            Err((4, 3))
        );
    }

    #[test]
    fn preserves_mode_for_blank_default_english_line() {
        let snapshot = snapshot("");
        let decision = super::classify_snapshot(&snapshot.text_snapshot).unwrap();

        assert_eq!(
            decision.reason,
            crate::classifier::DecisionReason::DefaultChinese
        );
        assert!(super::should_preserve_current_mode(
            &snapshot.text_snapshot,
            &decision
        ));
    }

    #[test]
    fn preserves_mode_for_uia_placeholder_only_default_english_line() {
        let snapshot = uia_snapshot("## --- []");
        let decision = super::classify_snapshot(&snapshot.text_snapshot).unwrap();

        assert_eq!(
            decision.reason,
            crate::classifier::DecisionReason::DefaultChinese
        );
        assert!(super::should_preserve_current_mode(
            &snapshot.text_snapshot,
            &decision
        ));
    }

    #[test]
    fn does_not_preserve_mode_for_win32_placeholder_only_default_english_line() {
        let snapshot = snapshot("## --- []");
        let decision = super::classify_snapshot(&snapshot.text_snapshot).unwrap();

        assert_eq!(
            decision.reason,
            crate::classifier::DecisionReason::DefaultChinese
        );
        assert!(!super::should_preserve_current_mode(
            &snapshot.text_snapshot,
            &decision
        ));
    }

    #[test]
    fn does_not_preserve_mode_for_non_blank_line() {
        let mut snapshot = snapshot("hello world");
        snapshot.text_snapshot.line_cursor_chars = 1;
        let decision = super::classify_snapshot(&snapshot.text_snapshot).unwrap();

        assert!(!super::should_preserve_current_mode(
            &snapshot.text_snapshot,
            &decision
        ));
    }

    #[test]
    fn chinese_mode_opens_ime() {
        assert!(ime_open_status_for_mode(InputMode::Chinese));
    }

    #[test]
    fn english_mode_closes_ime() {
        assert!(!ime_open_status_for_mode(InputMode::English));
    }

    #[test]
    fn chinese_mode_sets_native_conversion() {
        assert_eq!(
            ime_conversion_status_for_mode(InputMode::Chinese),
            IME_CMODE_NATIVE
        );
    }

    #[test]
    fn english_mode_clears_native_conversion() {
        assert_eq!(ime_conversion_status_for_mode(InputMode::English), 0);
    }

    #[test]
    fn open_status_true_maps_to_chinese_mode() {
        assert_eq!(input_mode_from_open_status(true), InputMode::Chinese);
    }

    #[test]
    fn open_status_false_maps_to_english_mode() {
        assert_eq!(input_mode_from_open_status(false), InputMode::English);
    }

    #[test]
    fn native_conversion_maps_to_chinese_mode() {
        assert_eq!(
            input_mode_from_conversion_status(IME_CMODE_NATIVE),
            InputMode::Chinese
        );
    }

    #[test]
    fn non_native_conversion_maps_to_english_mode() {
        assert_eq!(input_mode_from_conversion_status(0), InputMode::English);
    }

    #[test]
    fn ime_target_prefers_focus_hwnd() {
        let foreground = 1usize as *mut std::ffi::c_void;
        let focus = 2usize as *mut std::ffi::c_void;

        assert_eq!(resolve_ime_target_hwnd(foreground, focus), focus);
    }

    #[test]
    fn ime_target_falls_back_to_foreground_hwnd() {
        let foreground = 1usize as *mut std::ffi::c_void;

        assert_eq!(resolve_ime_target_hwnd(foreground, null_mut()), foreground);
    }

    #[test]
    fn watcher_skips_switch_when_mode_already_matches() {
        let controller = super::WindowsImeController::new();

        let outcome =
            maybe_switch_watcher_mode(&controller, Some(InputMode::Chinese), InputMode::Chinese);

        assert_eq!(outcome, Ok(WatcherSwitchOutcome::SkippedAlreadyMatched));
    }

    #[test]
    fn watcher_skips_switch_when_current_mode_is_unknown() {
        let controller = super::WindowsImeController::new();

        let outcome = maybe_switch_watcher_mode(&controller, None, InputMode::English);

        assert_eq!(outcome, Ok(WatcherSwitchOutcome::SkippedUnknownCurrentMode));
    }

    #[test]
    fn tray_runtime_control_toggles_pause_state() {
        let control = TrayRuntimeControl::new();

        assert!(!control.is_paused());
        assert!(control.toggle_paused());
        assert!(control.is_paused());
        assert!(!control.toggle_paused());
        assert!(!control.is_paused());
    }

    #[test]
    fn tray_command_id_uses_low_word() {
        assert_eq!(
            super::tray_command_id(Wparam::from(1001u16) << 16 | Wparam::from(42u16)),
            42
        );
    }

    #[test]
    fn tray_callback_event_uses_low_word() {
        assert_eq!(
            super::tray_callback_event((7isize << 16) | WM_RBUTTONUP as isize),
            WM_RBUTTONUP
        );
        assert_eq!(
            super::tray_callback_event((3isize << 16) | WM_LBUTTONUP as isize),
            WM_LBUTTONUP
        );
    }

    // --- extract_line_from_document tests ---

    #[test]
    fn extract_line_from_document_first_line() {
        let doc = "<script>\n\"这是一个用来测试的文件\"\n\n</script>";
        // Cursor at position 3 in first line: "<sc|ript>"
        let result = extract_line_from_document(doc, 3);
        assert_eq!(result, Some("<script>".to_string()));
    }

    #[test]
    fn extract_line_from_document_middle_line() {
        let doc = "<script>\n\"这是一个用来测试的文件\"\n\n</script>";
        // Cursor at position 5 in second line (offset 9 + 5 = 14): "这是|一个用来测试的文件"
        let result = extract_line_from_document(doc, 11);
        assert_eq!(result, Some("\"这是一个用来测试的文件\"".to_string()));
    }

    #[test]
    fn extract_line_from_document_empty_line() {
        let doc = "<script>\n\"这是一个用来测试的文件\"\n\n</script>";
        // Cursor on the empty third line (offset after second \n)
        let chars: Vec<char> = doc.chars().collect();
        let empty_line_offset = chars.iter().position(|&c| c == '\n').unwrap() + 1
            + chars[chars.iter().position(|&c| c == '\n').unwrap() + 1..]
                .iter()
                .position(|&c| c == '\n')
                .unwrap()
            + 1;
        let result = extract_line_from_document(doc, empty_line_offset);
        assert_eq!(result, Some("".to_string()));
    }

    #[test]
    fn extract_line_from_document_last_line() {
        let doc = "<script>\n\"这是一个用来测试的文件\"\n\n</script>";
        // Cursor in last line: "</script>" - use char offset
        let chars: Vec<char> = doc.chars().collect();
        let last_line_start = chars.iter().rposition(|&c| c == '\n').unwrap() + 1;
        let result = extract_line_from_document(doc, last_line_start + 3);
        assert_eq!(result, Some("</script>".to_string()));
    }

    #[test]
    fn extract_line_from_document_single_line() {
        let doc = "hello world";
        let result = extract_line_from_document(doc, 5);
        assert_eq!(result, Some("hello world".to_string()));
    }

    #[test]
    fn extract_line_from_document_cursor_at_start() {
        let doc = "line1\nline2";
        let result = extract_line_from_document(doc, 0);
        assert_eq!(result, Some("line1".to_string()));
    }

    #[test]
    fn extract_line_from_document_cursor_at_end() {
        let doc = "line1\nline2";
        let result = extract_line_from_document(doc, doc.chars().count());
        assert_eq!(result, Some("line2".to_string()));
    }

    #[test]
    fn extract_line_from_document_out_of_bounds() {
        let doc = "hello";
        let result = extract_line_from_document(doc, 100);
        assert_eq!(result, None);
    }

    #[test]
    fn vscode_extension_skips_non_vscode_processes() {
        // Should fail for non-VS Code processes
        let result = super::capture_vscode_extension_text("notepad.exe");
        assert!(result.is_err());
    }

    #[test]
    fn vscode_extension_tries_for_code_exe() {
        // Should attempt connection for Code.exe (will fail with pipe_connect_failed if extension not running)
        let result = super::capture_vscode_extension_text("Code.exe");
        assert!(result.is_err());
        // Verify it's the right error (pipe not available)
        match result {
            Err(super::TextReadResult::Failed(reason)) => {
                assert_eq!(reason, "pipe_connect_failed");
            }
            _ => panic!("Expected Failed error"),
        }
    }

    #[test]
    fn vscode_extension_tries_for_cursor_exe() {
        // Should attempt connection for Cursor.exe
        let result = super::capture_vscode_extension_text("Cursor.exe");
        assert!(result.is_err());
        match result {
            Err(super::TextReadResult::Failed(reason)) => {
                assert_eq!(reason, "pipe_connect_failed");
            }
            _ => panic!("Expected Failed error"),
        }
    }

    #[test]
    fn vscode_line_response_parses_correctly() {
        let json = r#"{"line": "const x = 1;", "cursor": 5, "lineNumber": 10, "totalLines": 100}"#;
        let response: super::VsCodeLineResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.line, "const x = 1;");
        assert_eq!(response.cursor, 5);
        assert_eq!(response.line_number, 10);
        assert_eq!(response.total_lines, 100);
    }

    #[test]
    fn vscode_line_response_handles_unicode() {
        let json = r#"{"line": "const 中文 = \"测试\";", "cursor": 8, "lineNumber": 3, "totalLines": 50}"#;
        let response: super::VsCodeLineResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.line, "const 中文 = \"测试\";");
        assert_eq!(response.cursor, 8);
    }
}
