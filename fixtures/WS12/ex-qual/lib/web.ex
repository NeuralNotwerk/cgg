defmodule MyApp.Web do
  alias MyApp.Accounts
  alias MyApp.Accounts.Cache, as: C
  alias MyApp.{Repo, Other}

  def show(id) do
    Accounts.get(id)
    Accounts.get(id, [])
    MyApp.Accounts.list()
    C.fetch(id)
    Repo.insert(id)
    Other.missing(id)
    :ets.new(:t, [])
    Enum.map([1], &Accounts.get/1)
    Unknown.Mod.f(id)
    Accounts.nope(id)
  end
end
