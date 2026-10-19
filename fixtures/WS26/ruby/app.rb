class Worker
  alias_method :run_now, :run_later      # run_later is referenced (old name)
  alias newer older                      # older is referenced

  def run_later; end
  def older; end
  def each_one(x); x; end
  def plain; end
  def asked; end                         # near-miss: only respond_to?
  def sent; end                          # dynuse screen only (send), no edge
  def orphan; end                        # control: dead
  def made; end                          # near-miss: define_method's new name

  def go(items)
    items.each(&method(:each_one))
    h = method(:plain)
    respond_to?(:asked)
    send(:sent)
    puts "made"
    h.call
  end
end

class Helper
  def self.build; new; end
  def fmt(x); x; end
end

def main
  w = Worker.new
  w.go([1])
  Helper.instance_method(:fmt)
end
main
