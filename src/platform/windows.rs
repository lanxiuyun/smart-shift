use crate::ime::InputMode;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::HWND as WinHwnd;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationTextRange, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start,
    TextUnit_Line, UIA_TextPatternId,
};

pub struct WindowsImeController;

impl WindowsImeController {
    pub fn new() -> Self {
        Self
    }

    pub fn switch_to(&self, mode: InputMode) -> Result<(), String> {
        Err(format!(
            "Windows IME switching is not implemented yet; requested mode={mode}"
        ))
    }
}

pub struct ForegroundWatcher {
    interval: Duration,
}

impl ForegroundWatcher {
    pub fn new(interval_ms: u64) -> Self {
        Self {
            interval: Duration::from_millis(interval_ms.max(50)),
        }
    }

    pub fn run(&self) -> Result<(), String> {
        println!("listener_started=true");
        println!("poll_interval_ms={}", self.interval.as_millis());

        let mut last_snapshot: Option<ForegroundSnapshot> = None;
        let mut last_error: Option<String> = None;

        loop {
            match capture_foreground_snapshot() {
                Ok(snapshot) => {
                    last_error = None;
                    let should_emit = should_emit_snapshot(last_snapshot.as_ref(), &snapshot);
                    if should_emit {
                        print_snapshot(&snapshot);
                    }
                    last_snapshot = Some(snapshot);
                }
                Err(error) => {
                    if last_error.as_ref() != Some(&error) {
                        println!("listener_warning={error}");
                        last_error = Some(error);
                    }
                }
            }
            thread::sleep(self.interval);
        }
    }
}

fn should_emit_snapshot(
    previous: Option<&ForegroundSnapshot>,
    current: &ForegroundSnapshot,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };

    if previous.foreground_hwnd != current.foreground_hwnd
        || previous.foreground_title != current.foreground_title
        || previous.focus_hwnd != current.focus_hwnd
        || previous.focus_class != current.focus_class
    {
        return true;
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
        return true;
    }

    if previous_text.document_len_utf16 != current_text.document_len_utf16 {
        return previous_text.source == "uia_text_pattern"
            && current_text.source == "uia_text_pattern"
            && previous_text.line_index != current_text.line_index;
    }

    if previous_text.line_text != current_text.line_text {
        return cursor_changed || caret_changed;
    }

    cursor_changed || caret_changed
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForegroundSnapshot {
    foreground_hwnd: isize,
    foreground_title: String,
    thread_id: u32,
    focus_hwnd: isize,
    focus_class: String,
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

    let text_snapshot = capture_text_snapshot(focus_hwnd, &focus_class);

    Ok(ForegroundSnapshot {
        foreground_hwnd: hwnd as isize,
        foreground_title: title,
        thread_id,
        focus_hwnd: focus_hwnd as isize,
        focus_class: focus_class.clone(),
        caret_hwnd: gui_info.hwndCaret as isize,
        caret_left: gui_info.rcCaret.left,
        caret_top: gui_info.rcCaret.top,
        caret_right: gui_info.rcCaret.right,
        caret_bottom: gui_info.rcCaret.bottom,
        text_snapshot,
    })
}

fn capture_text_snapshot(hwnd: HWND, class_name: &str) -> TextSnapshot {
    let mut attempts = Vec::new();

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

    match capture_app_adapter_text(hwnd, class_name) {
        Ok(mut snapshot) => {
            snapshot.attempts = attempts;
            return snapshot;
        }
        Err(result) => attempts.push(TextReadAttempt {
            source: "app_adapter",
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

    read_uia_text(hwnd).map_err(|_| TextReadResult::Unsupported("uia_text_pattern_unavailable"))
}

fn capture_app_adapter_text(_hwnd: HWND, class_name: &str) -> Result<TextSnapshot, TextReadResult> {
    let reason = match class_name {
        "Chrome_WidgetWin_1" => "electron_adapter_not_connected",
        _ => "no_app_adapter",
    };
    Err(TextReadResult::Unsupported(reason))
}

fn read_uia_text(hwnd: HWND) -> Result<TextSnapshot, windows::core::Error> {
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
        uia_range_to_line_snapshot(&document_range, &selected_range)
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

    let line_text = unsafe { line_range.GetText(-1)? }.to_string();
    let document_text = unsafe { document_range.GetText(-1)? }.to_string();
    let document_prefix_text = unsafe { document_prefix.GetText(-1)? }.to_string();
    let line_index = uia_line_index(document_range, &line_range)?;
    let line_prefix = unsafe { line_range.Clone()? };
    unsafe {
        line_prefix.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            selected_range,
            TextPatternRangeEndpoint_Start,
        )?
    };
    let line_prefix_text = unsafe { line_prefix.GetText(-1)? }.to_string();
    let document_len_utf16 = document_text.encode_utf16().count();
    let selection_start_utf16 = document_prefix_text.encode_utf16().count();
    let selection_end_utf16 = selection_start_utf16;
    let line_prefix_utf16 = line_prefix_text.encode_utf16().count();
    let line_prefix_chars = line_prefix_text.chars().count();

    Ok(TextSnapshot {
        source: "uia_text_pattern",
        document_len_utf16,
        selection_start_utf16,
        selection_end_utf16,
        line_index,
        line_cursor_utf16: line_prefix_utf16,
        line_cursor_chars: line_prefix_chars,
        line_text,
        attempts: Vec::new(),
    })
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

fn print_snapshot(snapshot: &ForegroundSnapshot) {
    println!(
        "window hwnd=0x{:X} thread_id={} title={}",
        snapshot.foreground_hwnd, snapshot.thread_id, snapshot.foreground_title
    );
    println!(
        "focus hwnd=0x{:X} class={}",
        snapshot.focus_hwnd, snapshot.focus_class
    );
    println!(
        "caret hwnd=0x{:X} rect=({}, {}, {}, {})",
        snapshot.caret_hwnd,
        snapshot.caret_left,
        snapshot.caret_top,
        snapshot.caret_right,
        snapshot.caret_bottom
    );

    if snapshot.text_snapshot.source != "unsupported" {
        let edit = &snapshot.text_snapshot;
        println!("text_source={}", edit.source);
        println!(
            "document_len_utf16={} selection utf16=({}, {}) line_index={} line_cursor_utf16={} line_cursor_chars={}",
            edit.document_len_utf16,
            edit.selection_start_utf16,
            edit.selection_end_utf16,
            edit.line_index,
            edit.line_cursor_utf16,
            edit.line_cursor_chars
        );
        println!("line_text={}", edit.line_text);
    } else {
        println!("line_text=unsupported");
        for attempt in &snapshot.text_snapshot.attempts {
            println!(
                "text_attempt source={} result={}",
                attempt.source,
                attempt.result.as_str()
            );
        }
    }
}

fn window_title(hwnd: HWND) -> String {
    let text = window_text_utf16(hwnd).unwrap_or_default();
    String::from_utf16_lossy(&text)
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
type Lparam = isize;
type Wparam = usize;
type Lresult = isize;
type LPARAM = Lparam;
type WPARAM = Wparam;
type LRESULT = Lresult;
type UINT = u32;

const WM_GETTEXT: UINT = 0x000D;
const WM_GETTEXTLENGTH: UINT = 0x000E;
const EM_GETSEL: UINT = 0x00B0;
const EM_GETLINE: UINT = 0x00C4;
const EM_LINEFROMCHAR: UINT = 0x00C9;
const EM_LINEINDEX: UINT = 0x00BB;
const EM_LINELENGTH: UINT = 0x00C1;

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

#[cfg(test)]
mod tests {
    use super::{ForegroundSnapshot, TextReadAttempt, TextSnapshot, utf16_units_to_char_index};

    fn snapshot(line_text: &str) -> ForegroundSnapshot {
        ForegroundSnapshot {
            foreground_hwnd: 1,
            foreground_title: "title".to_string(),
            thread_id: 1,
            focus_hwnd: 2,
            focus_class: "Edit".to_string(),
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

        assert!(!super::should_emit_snapshot(Some(&previous), &current));
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

        assert!(super::should_emit_snapshot(Some(&previous), &current));
    }

    #[test]
    fn emits_for_focus_change() {
        let previous = snapshot("abc");
        let mut current = snapshot("xyz");
        current.focus_hwnd = 99;

        assert!(super::should_emit_snapshot(Some(&previous), &current));
    }

    #[test]
    fn emits_cursor_move_after_suppressed_text_change_when_baseline_updates() {
        let previous = snapshot("abc");
        let mut typed = snapshot("abcd");
        typed.text_snapshot.selection_start_utf16 = 1;
        typed.text_snapshot.selection_end_utf16 = 1;
        typed.text_snapshot.line_cursor_utf16 = 1;
        typed.text_snapshot.line_cursor_chars = 1;
        assert!(!super::should_emit_snapshot(Some(&previous), &typed));

        let mut moved = typed.clone();
        moved.text_snapshot.selection_start_utf16 = 2;
        moved.text_snapshot.selection_end_utf16 = 2;
        moved.text_snapshot.line_cursor_utf16 = 2;
        moved.text_snapshot.line_cursor_chars = 2;
        moved.caret_left = 30;
        moved.caret_right = 31;

        assert!(super::should_emit_snapshot(Some(&typed), &moved));
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

        assert!(super::should_emit_snapshot(Some(&previous), &current));
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

        assert!(super::should_emit_snapshot(Some(&previous), &current));
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

        assert!(!super::should_emit_snapshot(Some(&previous), &current));
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

        assert!(super::should_emit_snapshot(Some(&previous), &current));
    }

    #[test]
    fn still_ignores_uia_same_line_text_edits() {
        let previous = uia_snapshot("abc");
        let mut current = uia_snapshot("abcd");
        current.text_snapshot.selection_start_utf16 = 4;
        current.text_snapshot.selection_end_utf16 = 4;
        current.text_snapshot.line_cursor_utf16 = 4;
        current.text_snapshot.line_cursor_chars = 4;

        assert!(!super::should_emit_snapshot(Some(&previous), &current));
    }
}
