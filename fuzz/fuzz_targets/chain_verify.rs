//! T-S25: the chain verifier over decoded entries (input format in `hy_fuzz::parse_chain`).
//! No input may panic. Whatever `extend` reaches must verify as that end, the same run
//! must be refused against any other end hash, and a run `extend` refuses never verifies.
#![no_main]

use hy_core::protocol::chain::{self, ChainError};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some(c) = hy_fuzz::parse_chain(data) else {
        return;
    };
    let claimed = chain::verify_extension(
        &c.user_id,
        c.start_seq,
        c.start_hash,
        &c.entries,
        c.end_seq,
        &c.end_hash,
    );
    match chain::extend(&c.user_id, c.start_seq, c.start_hash, &c.entries) {
        Ok((seq, hash)) => {
            if claimed.is_ok() {
                assert_eq!(
                    (seq, hash),
                    (c.end_seq, c.end_hash),
                    "only the real end verifies"
                );
            }
            chain::verify_extension(
                &c.user_id,
                c.start_seq,
                c.start_hash,
                &c.entries,
                seq,
                &hash,
            )
            .expect("the end extend reached verifies");
            let mut other = hash;
            other[31] ^= 1;
            assert_eq!(
                chain::verify_extension(
                    &c.user_id,
                    c.start_seq,
                    c.start_hash,
                    &c.entries,
                    seq,
                    &other
                ),
                Err(ChainError::Mismatch),
                "another end hash is refused"
            );
            assert!(seq >= c.start_seq);
            assert_eq!(seq - c.start_seq, c.entries.len() as u64);
        }
        Err(e) => assert_eq!(claimed, Err(e), "a run extend refuses never verifies"),
    }
});
