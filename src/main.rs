use std::process::ExitCode;

fn main() -> ExitCode {
    tracing::error!("runvane binary is not wired yet; see Phase 4 (CLI) and the main wiring commit.");
    eprintln!(
        "{} {} — {}",
        runvane::PRODUCT_NAME,
        runvane::VERSION,
        runvane::PRODUCT_TAGLINE
    );
    eprintln!("the binary entrypoint is assembled in a later phase");
    ExitCode::from(1)
}