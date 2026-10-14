class M {
  void F(List<Foo> xs) {
    xs.ForEach((Foo x) => x.Run());
    xs.ForEach((Foo? x) => { x.Run(); });
    xs.ForEach(x => x.Run());
  }
}
