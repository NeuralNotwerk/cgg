defmodule MyApp.Accounts do
  def get(id), do: id
  def get(id, opts), do: {id, opts}
  def list(), do: []
  defp secret(x), do: x
  def call_secret(x), do: secret(x)

  defmodule Cache do
    def fetch(k), do: k
  end

  def cached(k), do: Cache.fetch(k)
  def self_call(k), do: __MODULE__.get(k)
end
