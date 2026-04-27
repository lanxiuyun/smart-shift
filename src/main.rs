use smart_shift::app::{CliArgs, run};

fn main() {
    let args = CliArgs::from_env();

    if let Err(error) = run(args) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
