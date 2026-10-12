-module(xref).
-import(lists, [filter/2]).
-export([run/1]).
run(L) -> filter(fun(X) -> X end, L).
