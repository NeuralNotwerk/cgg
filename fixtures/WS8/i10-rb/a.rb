class Foo
  def run
    helper
    other 1
    self.helper
  end
  def helper; 1; end
  def other(x); x; end
end
