macro_rules! err {
    ($m:expr) => {
        return Err($m.to_string())
    };
}

fn parts(x: u32) -> Option<u32> { Some(x) }
fn key(x: u32) -> Option<u32> { Some(x) }
fn items() -> Vec<u32> { vec![] }
fn other(x: u32) -> u32 { x }

pub fn positive_if_let(x: u32) -> u32 {
    if let Some(parts) = parts(x) { return parts; }
    while let Some(key) = key(x) { return key; }
    for items in items() { let _ = items; }
    0
}

pub fn positive_macro(r: Result<u32, String>) -> Result<u32, String> {
    match r {
        Ok(v) => Ok(v),
        Err(err) => err!(err),
    }
}

// Near miss: the binding IS in scope in the body; a call through it is the value.
pub fn near_miss(f: Option<fn(u32) -> u32>) -> u32 {
    if let Some(other) = f { other(1) } else { other(2) }
}
