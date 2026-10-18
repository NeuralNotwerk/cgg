class Post < ActiveRecord::Base
  def initialize(a)
    super
  end
  def save
    super
  end
end
class Other
  def initialize; end
  def save; end
end
def top
  super
end
def make
  Thread.new { 1 }
  Other.new
end
