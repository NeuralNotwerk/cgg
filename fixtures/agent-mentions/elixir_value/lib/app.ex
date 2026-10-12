defmodule Ex.App do
  def run(xs) do
    ys = Enum.map(xs, &capture_target/1)
    zs = Enum.map(xs, &Ex.App.remote_capture/1)
    apply(Ex.App, :apply_target, [])
    apply(__MODULE__, :apply_mod_target, [])
    called_control()
    {ys, zs}
  end

  defp capture_target(x), do: x + 1
  def remote_capture(x), do: x
  def apply_target, do: 1
  defp apply_mod_target, do: 2
  defp dead_control, do: 0
  defp called_control, do: 3
end
