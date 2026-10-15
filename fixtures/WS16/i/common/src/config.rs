pub struct ConfigSource<T> { v: T }
impl<T: Clone> ConfigSource<T> {
    pub fn load(p: &str) -> Result<Self, String> { Err(p.to_string()) }
    pub fn get(&self) -> T { self.v.clone() }
}
