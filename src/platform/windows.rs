use crate::ime::InputMode;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use std::thread;
use std::time::Duration;

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
                    if last_snapshot.as_ref() != Some(&snapshot) {
                        print_snapshot(&snapshot);
                        last_snapshot = Some(snapshot);
                    }
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
    focused_edit: Option<FocusedEditSnapshot>,
    focused_edit_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FocusedEditSnapshot {
    selection_start_utf16: usize,
    selection_end_utf16: usize,
    line_index: usize,
    line_cursor_utf16: usize,
    line_cursor_chars: usize,
    line_text: String,
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

    let (focused_edit, focused_edit_error) = capture_focused_edit(focus_hwnd, &focus_class);

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
        focused_edit,
        focused_edit_error,
    })
}

fn capture_focused_edit(
    hwnd: HWND,
    class_name: &str,
) -> (Option<FocusedEditSnapshot>, Option<String>) {
    if hwnd.is_null() || !is_supported_edit_class(class_name) {
        return (None, Some("unsupported_control_class".to_string()));
    }

    let (selection_start_utf16, selection_end_utf16) = match edit_selection(hwnd) {
        Some(value) => value,
        None => return (None, Some("selection_read_failed".to_string())),
    };
    let line_index = match edit_line_from_char(hwnd, selection_start_utf16) {
        Some(value) => value,
        None => return (None, Some("line_from_char_failed".to_string())),
    };
    let line_start_utf16 = match edit_line_index(hwnd, line_index) {
        Some(value) => value,
        None => return (None, Some("line_index_failed".to_string())),
    };
    let line_len_utf16 = match edit_line_length(hwnd, line_start_utf16) {
        Some(value) => value,
        None => return (None, Some("line_length_failed".to_string())),
    };
    if selection_start_utf16 < line_start_utf16 {
        return (None, Some("line_range_out_of_bounds".to_string()));
    }

    let line_utf16 = match edit_get_line(hwnd, line_index, line_len_utf16) {
        Some(value) => value,
        None => return (None, Some("line_read_failed".to_string())),
    };
    let line_text = String::from_utf16_lossy(&line_utf16);
    let line_cursor_utf16 = selection_start_utf16 - line_start_utf16;
    let line_cursor_chars = match utf16_units_to_char_index(&line_utf16, line_cursor_utf16) {
        Some(value) => value,
        None => return (None, Some("cursor_utf16_to_char_failed".to_string())),
    };

    (
        Some(FocusedEditSnapshot {
            selection_start_utf16,
            selection_end_utf16,
            line_index,
            line_cursor_utf16,
            line_cursor_chars,
            line_text,
        }),
        None,
    )
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

    if let Some(edit) = &snapshot.focused_edit {
        println!(
            "selection utf16=({}, {}) line_index={} line_cursor_utf16={} line_cursor_chars={}",
            edit.selection_start_utf16,
            edit.selection_end_utf16,
            edit.line_index,
            edit.line_cursor_utf16,
            edit.line_cursor_chars
        );
        println!("line_text={}", edit.line_text);
    } else {
        println!("line_text=unsupported");
        if let Some(error) = &snapshot.focused_edit_error {
            println!("line_text_error={error}");
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
    use super::utf16_units_to_char_index;

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
}
