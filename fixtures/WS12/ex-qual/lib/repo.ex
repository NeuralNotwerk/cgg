defmodule MyApp.Repo do
  def insert(x), do: x
end
defmodule Decoy do
  def get(id), do: id
  def fetch(k), do: k
  def insert(x), do: x
  def f(x), do: x
end
