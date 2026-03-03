use std::process;

fn main() {
    if let Err(e) = jupytext::cli::run_cli() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}
