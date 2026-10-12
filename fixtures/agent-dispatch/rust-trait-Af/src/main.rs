pub trait Processor {
    fn process(&self);
    fn unused_op(&self);
}

pub struct Alpha;
impl Processor for Alpha {
    fn process(&self) { println!("Alpha"); }
    fn unused_op(&self) { println!("u"); }
}

pub struct Beta;
impl Processor for Beta {
    fn process(&self) { println!("Beta"); }
    fn unused_op(&self) { println!("u"); }
}

struct Holder {
    field: Box<dyn Processor>,
}
impl Holder {
    fn run_field(&self) {
        self.field.process();
    }
}

fn main() {
    Holder { field: Box::new(Alpha) }.run_field();
    Holder { field: Box::new(Beta) }.run_field();
}
