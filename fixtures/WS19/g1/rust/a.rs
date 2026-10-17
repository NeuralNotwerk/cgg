struct Service;
impl Service {
    fn run(&self) {}
}
fn main() {
    let service = mk();
    service.run();
}
