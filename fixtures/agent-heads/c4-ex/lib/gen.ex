defmodule MyApp.Gen do
  defmacro defgetter(name) do
    quote do
      def unquote(:"get_#{name}")(), do: unquote(name)
    end
  end
end
