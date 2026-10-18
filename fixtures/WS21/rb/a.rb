class Base
  def initialize(x)
  end
  def run
  end
end
class Coll < Base
  def initialize(y)
    super
  end
  def run
    super
  end
end
def main
  c = Coll.new(1)
end
