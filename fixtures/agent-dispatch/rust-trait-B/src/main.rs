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

pub struct Gamma;
impl Processor for Gamma {
    fn process(&self) { println!("Gamma"); }
    fn unused_op(&self) { println!("u"); }
}

pub struct Delta;
impl Processor for Delta {
    fn process(&self) { println!("Delta"); }
    fn unused_op(&self) { println!("u"); }
}

pub struct Epsilon;
impl Processor for Epsilon {
    fn process(&self) { println!("Epsilon"); }
    fn unused_op(&self) { println!("u"); }
}

pub struct Zeta;
impl Processor for Zeta {
    fn process(&self) { println!("Zeta"); }
    fn unused_op(&self) { println!("u"); }
}

fn run<T: Processor>(p: &T) {
    p.process();
}

fn main() {
    run(&Alpha);
    run(&Beta);
    run(&Gamma);
    run(&Delta);
    run(&Epsilon);
    run(&Zeta);
}
