#![no_main]

use libfuzzer_sys::fuzz_target;
use runvane::events::EventKind;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let kinds = [
            EventKind::RunStarted,
            EventKind::RunSucceeded,
            EventKind::RunFailed,
            EventKind::RunTimedOut,
            EventKind::RunCancelled,
        ];
        for k in kinds {
            let _ = s == k.code() || s == format!("{:?}", k);
        }
    }
});
