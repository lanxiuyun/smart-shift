use crate::classifier::{DecisionReason, classify};
use crate::context::LineContext;
use crate::ime::InputMode;
use crate::platform::windows::{ForegroundWatcher, WindowsImeController};
use std::env;
use std::error::Error;
use std::fmt;

pub struct CliArgs {
    pub line: String,
    pub cursor: usize,
    pub apply: bool,
    pub listen: bool,
    pub interval_ms: u64,
}

impl CliArgs {
    pub fn from_env() -> Self {
        let mut line = String::new();
        let mut cursor = 0usize;
        let mut apply = false;
        let mut listen = false;
        let mut interval_ms = 250u64;

        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--line" => {
                    line = args.next().unwrap_or_default();
                }
                "--cursor" => {
                    cursor = args
                        .next()
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(0);
                }
                "--apply" => {
                    apply = true;
                }
                "--listen" => {
                    listen = true;
                }
                "--interval-ms" => {
                    interval_ms = args
                        .next()
                        .and_then(|value| value.parse::<u64>().ok())
                        .unwrap_or(250);
                }
                _ => {}
            }
        }

        Self {
            line,
            cursor,
            apply,
            listen,
            interval_ms,
        }
    }
}

#[derive(Debug)]
pub enum AppError {
    MissingLine,
    InvalidCursor { cursor: usize, text_len: usize },
    Platform(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLine => write!(f, "missing required argument: --line"),
            Self::InvalidCursor { cursor, text_len } => {
                write!(
                    f,
                    "cursor {cursor} is out of bounds for text length {text_len}"
                )
            }
            Self::Platform(message) => write!(f, "{message}"),
        }
    }
}

impl Error for AppError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyOutcome {
    Applied,
    SkippedAlreadyMatched,
    SkippedUnknownCurrentMode,
}

pub fn run(args: CliArgs) -> Result<(), AppError> {
    if args.listen || args.line.is_empty() {
        return run_listener(args.interval_ms);
    }

    let context = LineContext::new(args.line, args.cursor)
        .map_err(|(cursor, text_len)| AppError::InvalidCursor { cursor, text_len })?;
    let decision = classify(&context);

    print_decision(&context, decision.mode, &decision.reason);
    print_current_ime_mode();

    if args.apply {
        let controller = WindowsImeController::new();
        let before_mode = controller.current_mode().ok();
        let apply_outcome = maybe_apply_mode(&controller, before_mode, decision.mode)
            .map_err(AppError::Platform)?;
        print_apply_outcome(apply_outcome);
        print_ime_mode_result("ime_mode_before_apply", before_mode);
        print_ime_mode_result("ime_mode_after_apply", controller.current_mode().ok());
    } else {
        println!("applied=false");
    }

    Ok(())
}

fn run_listener(interval_ms: u64) -> Result<(), AppError> {
    let watcher = ForegroundWatcher::new(interval_ms);
    watcher.run().map_err(AppError::Platform)
}

fn print_decision(context: &LineContext, mode: InputMode, reason: &DecisionReason) {
    println!("line={}", context.line());
    println!("cursor={}", context.cursor());
    println!("target_mode={mode}");
    println!("reason={reason}");
}

fn print_current_ime_mode() {
    let controller = WindowsImeController::new();
    match controller.current_mode() {
        Ok(mode) => println!("current_ime_mode={mode}"),
        Err(error) => {
            println!("current_ime_mode=unknown");
            println!("ime_read_error={error}");
        }
    }
}

fn print_ime_mode_result(label: &str, mode: Option<InputMode>) {
    match mode {
        Some(mode) => println!("{label}={mode}"),
        None => println!("{label}=unknown"),
    }
}

fn maybe_apply_mode(
    controller: &WindowsImeController,
    current_mode: Option<InputMode>,
    target_mode: InputMode,
) -> Result<ApplyOutcome, String> {
    match current_mode {
        Some(mode) if mode == target_mode => Ok(ApplyOutcome::SkippedAlreadyMatched),
        Some(_) => {
            controller.switch_to(target_mode)?;
            Ok(ApplyOutcome::Applied)
        }
        None => Ok(ApplyOutcome::SkippedUnknownCurrentMode),
    }
}

fn print_apply_outcome(outcome: ApplyOutcome) {
    match outcome {
        ApplyOutcome::Applied => {
            println!("applied=true");
            println!("apply_reason=mode_changed");
        }
        ApplyOutcome::SkippedAlreadyMatched => {
            println!("applied=false");
            println!("apply_reason=already_matched");
        }
        ApplyOutcome::SkippedUnknownCurrentMode => {
            println!("applied=false");
            println!("apply_reason=current_mode_unknown");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ApplyOutcome, maybe_apply_mode};
    use crate::ime::InputMode;
    use crate::platform::windows::WindowsImeController;

    #[test]
    fn skips_apply_when_mode_already_matches() {
        let controller = WindowsImeController::new();

        let outcome = maybe_apply_mode(&controller, Some(InputMode::Chinese), InputMode::Chinese);

        assert_eq!(outcome, Ok(ApplyOutcome::SkippedAlreadyMatched));
    }

    #[test]
    fn skips_apply_when_current_mode_is_unknown() {
        let controller = WindowsImeController::new();

        let outcome = maybe_apply_mode(&controller, None, InputMode::English);

        assert_eq!(outcome, Ok(ApplyOutcome::SkippedUnknownCurrentMode));
    }
}
