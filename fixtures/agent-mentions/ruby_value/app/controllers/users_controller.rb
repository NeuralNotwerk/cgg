class UsersController < ApplicationController
  before_action :authenticate
  after_action :log_it

  def index
    called_control
    [1, 2].each(&method(:each_target))
    m = method(:method_obj_target)
    send(:send_target)
    public_send(:public_send_target)
    m.call
  end

  private

  def authenticate; end
  def log_it; end
  def each_target(x); end
  def method_obj_target; end
  def send_target; end
  def public_send_target; end
  def dead_control; end
  def called_control; end
end
