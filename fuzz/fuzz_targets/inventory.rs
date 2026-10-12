#![no_main]

use libfuzzer_sys::fuzz_target;
mod inventory_check;
fuzz_target!(|input: &[u8]| inventory_check::check(input));
