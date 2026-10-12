module Gen
for name in (:alpha, :beta)
    @eval $(Symbol("get_", name))() = $(QuoteNode(name))
end
macro defgetter(n)
    fname = Symbol("make_", n)
    esc(:($fname() = 1))
end
@defgetter widget
real_fn() = 1
function use_it()
    get_alpha()
    make_widget()
    real_fn()
end
end
