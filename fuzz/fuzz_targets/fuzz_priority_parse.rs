#![no_main]

use libfuzzer_sys::fuzz_target;
use runvane::domain::status::Priority;
use std::str::FromStr;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = Priority::from_str(s);
    }
});
