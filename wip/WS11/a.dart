void f(List<Foo> xs) {
  xs.forEach((x) => x.run());
  xs.forEach((Foo x) { x.run(); });
  y!.run();
}
