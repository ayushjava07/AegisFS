#![no_main]

use libfuzzer_sys::fuzz_target;
use runvane::api::payloads::RunQuery;

fuzz_target!(|data: &[u8]| {
    if let Ok(query) = serde_json::from_slice::<RunQuery>(data) {
        let _ = query.into_filter();
    }
});
