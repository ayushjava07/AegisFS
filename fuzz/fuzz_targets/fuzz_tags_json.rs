#![no_main]

use libfuzzer_sys::fuzz_target;
use runvane::domain::validation::validate_run_input;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    if let Ok(val) = serde_json::from_slice::<Value>(data) {
        let _ = validate_run_input(&val);
    }
});
