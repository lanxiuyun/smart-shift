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

pub fn run(args: CliArgs) -> Result<(), AppError> {
    if args.listen {
        return run_listener(args.interval_ms);
    }

    if args.line.is_empty() {
        return Err(AppError::MissingLine);
    }

    let context = LineContext::new(args.line, args.cursor)
        .map_err(|(cursor, text_len)| AppError::InvalidCursor { cursor, text_len })?;
    let decision = classify(&context);

    print_decision(&context, decision.mode, &decision.reason);

    if args.apply {
        let controller = WindowsImeController::new();
        controller
            .switch_to(decision.mode)
            .map_err(AppError::Platform)?;
        println!("applied=true");
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
