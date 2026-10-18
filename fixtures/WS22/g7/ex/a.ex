defmodule Shape do
  @callback area(term) :: number
end
defmodule Sq do
  @behaviour Shape
  def area(_), do: 1
end
defmodule Ci do
  @behaviour Shape
  def area(_), do: 3
end
defmodule Use do
  def run(m, x), do: m.area(x)
end
