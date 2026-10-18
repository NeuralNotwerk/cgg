pub trait Shape { fn area(&self) -> f64; fn twice(&self) -> f64 { 2.0 * self.area() } }
pub struct Sq; pub struct Ci;
impl Shape for Sq { fn area(&self) -> f64 { 1.0 } }
impl Shape for Ci { fn area(&self) -> f64 { 3.0 } }
pub fn run(h: &Holder) { h.item.area(); }
