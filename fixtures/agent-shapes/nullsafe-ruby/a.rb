class Foo
  def zorkle; 1; end
end
def ctl_call(f)
  f.zorkle
end
def null_safe(f)
  f&.zorkle
end
