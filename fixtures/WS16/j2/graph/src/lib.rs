use kb_common::{ChangeMeter};
pub fn upsert(meter: Option<&ChangeMeter>, empty: bool) {
    if let Some(m) = meter {
        m.record_only(1);
    }
    if let (Some(m), false) = (meter, empty) {
        m.record_only(2);
    }
    let direct: &ChangeMeter = meter.unwrap();
    direct.record_only(3);
}
pub struct MeasurementRow { pub v: u32 }
pub fn rows() -> u32 {
    let m: MeasurementRow = MeasurementRow { v: 1 };
    m.v
}
