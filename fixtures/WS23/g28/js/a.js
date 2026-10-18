class Svc { run() {} }
class Other { run() {} }
function f1() {
  const t = new Svc();
  t.run();
  const u = make();
  u.run();
  let v = new Svc();
  v.run();
  var w = null;
  w.run();
}
// same local name typed differently in two functions
function a() { const x = new Svc(); x.run(); }
function b() { const x = new Other(); x.run(); }
// reassigned local: untyped
function c() { let y = new Svc(); y.run(); y = new Other(); y.run(); }
// shadowing inner scope and closure parameter
function d() {
  const z = new Svc();
  z.run();
  { const z = new Other(); z.run(); }
  [1].forEach((z) => z.run());
  z.run();
  const g = () => { z.run(); };
}
// annotated let reassigned keeps its declared type
function e() { let q = new Svc(); q = new Svc(); q.run(); }
function make() { return null; }
