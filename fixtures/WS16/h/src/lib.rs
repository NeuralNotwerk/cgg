pub mod id;
use crate::id::DocId;
pub struct Doc { pub doc_id: String }
pub fn by_param(id: String) -> DocId {
    DocId::from(id)
}
pub fn by_clone(name: &str) -> DocId {
    let owned: String = name.to_string();
    DocId::from(owned.clone())
}
pub fn by_str(name: &str) -> DocId {
    DocId::from(name)
}
pub fn unknown(d: &Doc) -> DocId {
    DocId::from(d.doc_id.clone())
}
