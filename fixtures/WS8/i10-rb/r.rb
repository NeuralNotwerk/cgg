class Foo
  def run(a)
    x = 1
    helper
    x
    a
    puts x
    y = helper + x
  end
  def helper; 1; end
end
