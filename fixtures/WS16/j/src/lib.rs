pub mod meter;
use crate::meter::{ChangeMeter, Handle};
pub fn upsert(meter: Option<&ChangeMeter>, empty: bool) {
    if let Some(m) = meter {
        m.record_only(1);
    }
    if let (Some(m), false) = (meter, empty) {
        m.record_only(2);
    }
}
pub fn routes(h: Option<&Handle>) -> u32 {
    h.map(|h| {
        let o = h.snapshot();
        o.entity_types()
    }).unwrap_or(0)
}
pub fn plain(h: &Handle) -> u32 {
    let o = h.snapshot();
    o.entity_types()
}
