-module(asn1ct_constructed_per).
-export([run/1, gen_dec_prim/1, stdlib/1]).
%% `m:f` names module m: never asn1ct_gen_ber_bin_v2's same-named function,
%% and never this file's own gen_dec_prim either.
run(X) -> asn1ct_gen_per:gen_dec_prim(X).
gen_dec_prim(X) -> X.
%% A module the tree lacks: no edge.
stdlib(X) -> lists:reverse(X).
