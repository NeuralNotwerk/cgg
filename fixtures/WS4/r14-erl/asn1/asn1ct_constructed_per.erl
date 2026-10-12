-module(asn1ct_constructed_per).
-export([run/1]).
run(X) -> asn1ct_gen_per:gen_dec_prim(X).
