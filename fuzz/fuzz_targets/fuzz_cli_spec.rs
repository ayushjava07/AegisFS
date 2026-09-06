#![no_main]

use libfuzzer_sys::fuzz_target;
use runvane::cli::Cli;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let args: Vec<&str> = std::iter::once("runvane")
            .chain(s.split_whitespace())
            .collect();
        let _ = Cli::try_parse_from(args);
    }
});
