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

fn run(p: &dyn Processor) {
    p.process();
}

fn main() {
    run(&Alpha);
    run(&Beta);
}
