use std::process::ExitCode;

use clap::Parser;
use runvane::cli::Cli;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::try_parse().unwrap_or_else(|err| {
        // clap already printed an error + usage; match its exit semantics.
        err.exit()
    });

    let filter = std::env::var("RUST_LOG")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "info".to_owned());
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();

    match runvane::cli::execute(&cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "command failed");
            eprintln!("runvane: {err}");
            ExitCode::from(1)
        }
    }
}