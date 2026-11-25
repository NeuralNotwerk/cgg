//! Print every language plugin id, one per line.
//!
//! `cgg::plugin_ids()` is the authoritative list (the same one the Python
//! and Node bindings return from `languages()`). The CLI deliberately has
//! no flag for it, so scripts that must cover every language —
//! `scripts/windows-parity.sh` — read it from here:
//!
//! ```text
//! cargo run -q --release -p cgg --example plugin_ids
//! ```

fn main() {
    for id in cgg::plugin_ids() {
        println!("{id}");
    }
}
