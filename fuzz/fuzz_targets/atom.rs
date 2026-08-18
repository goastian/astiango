#![no_main]

use astiango::feed;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    let _ = feed::parse(data, feed::FeedKind::Atom);
});
