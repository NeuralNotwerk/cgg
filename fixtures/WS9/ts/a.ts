function f<T>(x: T): T { return x; }
class B { m<T>(x: T): T { return x; } }
function caller(b: B) {
  f<number>(1);
  b.m<string>("a");
  f(2);
}
