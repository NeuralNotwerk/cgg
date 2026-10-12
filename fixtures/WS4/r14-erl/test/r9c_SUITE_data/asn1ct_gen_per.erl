-module(asn1ct_gen_per).
%% A copy kept as test data: never the target from outside it.
-export([gen_dec_prim/1]).
gen_dec_prim(X) -> {old, X}.
