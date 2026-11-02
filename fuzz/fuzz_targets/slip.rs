//! T-S25: the pairing slip parser (`hy1-<host>-<port>-<fp>-<secret>-<check>`), as pasted or
//! typed by the user. No input may panic; a slip that parses must print back to slips
//! (canonical and grouped) that parse to the same host, port, pin and secret.
#![no_main]

use hy_core::protocol::slip::{self, Slip};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = slip::decode_secret(text);
    let _ = slip::host_ok(text);
    let Ok(s) = Slip::parse(text) else {
        return;
    };
    assert!(slip::host_ok(&s.host), "a parsed slip has a valid host");
    assert_ne!(s.port, 0, "a parsed slip has a port");
    for printed in [s.canonical(), s.printable()] {
        let back = Slip::parse(printed.expose()).expect("a printed slip parses");
        assert_eq!(back.host, s.host);
        assert_eq!(back.port, s.port);
        assert_eq!(back.fingerprint, s.fingerprint);
        assert_eq!(back.secret.expose(), s.secret.expose());
    }
});
