// A workspace type that shares its name with an external crate's wrapper.
pub struct Data {
    n: u32,
}

impl Data {
    pub fn pool(&self) -> u32 {
        self.n
    }
}

pub struct Bytes(Vec<u8>);

impl From<u8> for Bytes { fn from(x: u8) -> Self { Bytes(vec![x]) } }
impl From<u16> for Bytes { fn from(x: u16) -> Self { Bytes(vec![x as u8]) } }
impl From<u32> for Bytes { fn from(x: u32) -> Self { Bytes(vec![x as u8]) } }
impl From<u64> for Bytes { fn from(x: u64) -> Self { Bytes(vec![x as u8]) } }
impl From<i8> for Bytes { fn from(x: i8) -> Self { Bytes(vec![x as u8]) } }
impl From<i16> for Bytes { fn from(x: i16) -> Self { Bytes(vec![x as u8]) } }
impl From<String> for Bytes { fn from(x: String) -> Self { Bytes(x.into_bytes()) } }

pub fn make_bytes(s: String) -> Bytes {
    Bytes::from(s)
}

pub fn make_one() -> Bytes {
    <Bytes as From<u8>>::from(1)
}
