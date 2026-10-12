defmodule MyApp.App2 do
  alias MyApp.Util
  def run2(x) do
    Util.decode("x")
    MyApp.Util.get("u")
    x.request("u")
  end
end
