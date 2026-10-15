pub struct ChangeMeter;
impl ChangeMeter {
    pub fn record_only(&self, n: u32) {}
}
pub struct Ontology;
impl Ontology {
    pub fn entity_types(&self) -> u32 { 0 }
}
pub struct Handle;
impl Handle {
    pub fn snapshot(&self) -> std::sync::Arc<Ontology> { std::sync::Arc::new(Ontology) }
}
