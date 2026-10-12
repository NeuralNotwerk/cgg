-module(lists).
-export([filter/2]).
filter(F, L) -> [X || X <- L, F(X)].
