class Kid < Base
  include Helpers
  def run
    helped
    inherited_m
    unrelated
    top_level_fn
  end
end
