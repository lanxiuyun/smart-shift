mod ime;
mod platform;

use ime::InputMode;
use platform::windows::{current_ime_target_hwnd, write_ime_mode, WindowsSingleInstance};
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::Write;
use std::thread;
use std::time::Duration;

const PIPE_NAME: &str = r"\\.\pipe\smart-shift-daemon";

fn main() {
    let _instance = match WindowsSingleInstance::acquire("smart-shift-daemon") {
        Ok(i) => i,
        Err(e) => {
            log(&format!("already running: {e}"));
            std::process::exit(1);
        }
    };

    log("smart-shift daemon started");

    loop {
        if let Err(e) = handle_one_connection() {
            log(&format!("connection error: {e}"));
            thread::sleep(Duration::from_millis(100));
        }
    }
}

fn handle_one_connection() -> Result<(), String> {
    let pipe = create_named_pipe()?;
    let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) };
    if connected == 0 {
        let err = unsafe { GetLastError() };
        // ERROR_PIPE_CONNECTED = 535
        if err != 535 {
            unsafe { CloseHandle(pipe) };
            return Err(format!("ConnectNamedPipe failed: {err}"));
        }
    }

    let mut buf = [0u8; 1024];
    let mut read = 0u32;
    let ok = unsafe {
        ReadFile(
            pipe,
            buf.as_mut_ptr() as *mut c_void,
            buf.len() as u32,
            &mut read,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        unsafe { CloseHandle(pipe) };
        return Err(format!("ReadFile failed: {err}"));
    }

    let req = String::from_utf8_lossy(&buf[..read as usize])
        .trim()
        .to_string();
    log(&format!("> {req}"));

    let resp = match req.as_str() {
        "PING" => "PONG\n",
        "SWITCH chinese" => match switch_ime(InputMode::Chinese) {
            Ok(()) => "OK\n",
            Err(e) => {
                log(&format!("switch chinese err: {e}"));
                "ERR\n"
            }
        },
        "SWITCH english" => match switch_ime(InputMode::English) {
            Ok(()) => "OK\n",
            Err(e) => {
                log(&format!("switch english err: {e}"));
                "ERR\n"
            }
        },
        _ => "ERR unknown\n",
    };

    let bytes = resp.as_bytes();
    let mut written = 0u32;
    unsafe {
        WriteFile(
            pipe,
            bytes.as_ptr() as *const c_void,
            bytes.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        );
        FlushFileBuffers(pipe);
        DisconnectNamedPipe(pipe);
        CloseHandle(pipe);
    }

    log(&format!("< {}", resp.trim()));
    Ok(())
}

fn switch_ime(mode: InputMode) -> Result<(), String> {
    let hwnd = current_ime_target_hwnd()?;
    write_ime_mode(hwnd, mode)
}

fn log(msg: &str) {
    let path = std::env::temp_dir().join("smart-shift-daemon.log");
    let line = format!("{}\n", msg);
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

fn create_named_pipe() -> Result<*mut c_void, String> {
    let name: Vec<u16> = PIPE_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    let h = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            1,
            1024,
            1024,
            0,
            std::ptr::null_mut(),
        )
    };
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        return Err(format!(
            "CreateNamedPipeW failed: {}",
            unsafe { GetLastError() }
        ));
    }
    Ok(h)
}

// Named Pipe FFI
const PIPE_ACCESS_DUPLEX: u32 = 0x00000003;
const PIPE_TYPE_MESSAGE: u32 = 0x00000004;
const PIPE_READMODE_MESSAGE: u32 = 0x00000002;
const PIPE_WAIT: u32 = 0x00000000;
const INVALID_HANDLE_VALUE: *mut c_void = (-1isize) as *mut c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateNamedPipeW(
        lpName: *const u16,
        dwOpenMode: u32,
        dwPipeMode: u32,
        nMaxInstances: u32,
        nOutBufferSize: u32,
        nInBufferSize: u32,
        nDefaultTimeOut: u32,
        lpSecurityAttributes: *mut c_void,
    ) -> *mut c_void;
    fn ConnectNamedPipe(hNamedPipe: *mut c_void, lpOverlapped: *mut c_void) -> i32;
    fn DisconnectNamedPipe(hNamedPipe: *mut c_void) -> i32;
    fn ReadFile(
        hFile: *mut c_void,
        lpBuffer: *mut c_void,
        nNumberOfBytesToRead: u32,
        lpNumberOfBytesRead: *mut u32,
        lpOverlapped: *mut c_void,
    ) -> i32;
    fn WriteFile(
        hFile: *mut c_void,
        lpBuffer: *const c_void,
        nNumberOfBytesToWrite: u32,
        lpNumberOfBytesWritten: *mut u32,
        lpOverlapped: *mut c_void,
    ) -> i32;
    fn FlushFileBuffers(hFile: *mut c_void) -> i32;
    fn CloseHandle(hObject: *mut c_void) -> i32;
    fn GetLastError() -> u32;
}
