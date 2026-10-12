defmodule MyApp.App do
  def run do
    Jason.decode("x")
    HTTPoison.get("u")
    :hackney.request("u")
    Enum.map([1], fn x -> x end)
  end
end
