pub struct DocId(String);
impl From<String> for DocId {
    fn from(s: String) -> Self { DocId(s) }
}
impl From<&str> for DocId {
    fn from(s: &str) -> Self { DocId(s.to_string()) }
}
