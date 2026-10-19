-module(worker).

start_link() -> ok.
loop(N) -> N.
stop(R) -> R.
stop(R, T) -> {R, T}.
unused() -> ok.
tick(X) -> X.
