defmodule A do
  def run(xs) do
    Enum.map(xs, &f/1)
    Enum.map(xs, &A.g/1)
  end
  def f(x), do: x
  def g(x), do: x
end
