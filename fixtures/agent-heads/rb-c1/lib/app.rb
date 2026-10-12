require 'json'
require 'httparty'
require_relative 'util'

class App
  def run
    JSON.parse("{}")
    HTTParty.get("u")
    Faraday.post("u")
    Time.now.strftime("%Y")
  end
end
