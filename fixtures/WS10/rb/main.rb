class Main
  def run(f)
    Foo.new.m
    (Foo.new).m
    f&.m
  end
end
