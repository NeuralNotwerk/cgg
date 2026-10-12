-module(user).
-export([run/1]).
%% Two copies of asn1ct_gen_per: the shipped one, not the test data.
run(X) -> asn1ct_gen_per:gen_dec_prim(X).
