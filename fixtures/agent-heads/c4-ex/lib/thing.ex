defmodule MyApp.Thing do
  require MyApp.Gen
  MyApp.Gen.defgetter(:color)
  def real_fn(), do: 1
  def use_it() do
    get_color()
    real_fn()
  end
end
