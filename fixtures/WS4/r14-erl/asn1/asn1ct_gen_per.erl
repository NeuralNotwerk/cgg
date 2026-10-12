-module(asn1ct_gen_per).
-export([gen_dec_prim/1, again/1, local_twin/1]).
gen_dec_prim(X) -> X.
%% A call qualified with the file's own module binds the file's own function.
again(X) -> asn1ct_gen_per:gen_dec_prim(X).
%% A bare call stays in the module.
local_twin(X) -> gen_dec_prim(X).
