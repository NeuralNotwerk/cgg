-module(main).
-export([main/0]).
-record(st, {cb = fun tick/1}).

main() ->
    lists:map(fun step/1, [1]),
    F = fun helper/2,
    G = fun worker:stop/1,
    S = {worker, start_link, []},
    spawn(worker, loop, [1]),
    D = {ok, compute},
    {F, G, S, D}.

step(X) -> X.
helper(X) -> X.
compute() -> ok.
tick(X) -> X.
