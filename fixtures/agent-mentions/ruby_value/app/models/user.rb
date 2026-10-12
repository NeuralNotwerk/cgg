class User < ApplicationRecord
  validate :check_x
  after_commit :notify_x
  before_save :normalize

  private

  def check_x; end
  def notify_x; end
  def normalize; end
  def model_dead; end
end
