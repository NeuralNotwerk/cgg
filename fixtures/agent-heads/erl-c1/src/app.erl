-module(app).
-export([run/0]).
run() ->
    jsx:decode(<<"x">>),
    hackney:request(get),
    lists:map(fun(X) -> X end, [1]).
