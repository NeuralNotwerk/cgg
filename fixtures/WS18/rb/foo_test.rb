require "minitest/autorun"
class FooTest < Minitest::Test
  def setup; end
  def test_a; helper; end
  def helper; end
  def unused_r; end
end
