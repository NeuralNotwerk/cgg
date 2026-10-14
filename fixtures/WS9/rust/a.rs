fn f<T>(x: T) -> T { x }
struct B;
impl B {
    fn m<T>(&self, x: T) -> T { x }
    fn s<T>(x: T) -> T { x }
}
fn caller() {
    let b = B;
    f::<i32>(1);
    b.m::<i32>(2);
    B::s::<i32>(3);
    f(4);
}
