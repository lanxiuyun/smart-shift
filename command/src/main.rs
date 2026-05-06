#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use smart_shift::app::{CliArgs, run};
use smart_shift::platform::windows::show_error_dialog;

fn main() {
    let args = CliArgs::from_env();
    let runs_in_background_app_mode = args.runs_in_background_app_mode();

    if let Err(error) = run(args) {
        eprintln!("error: {error}");
        if runs_in_background_app_mode {
            show_error_dialog("smart-shift failed to start", &error.to_string());
        }
        std::process::exit(1);
    }
}
