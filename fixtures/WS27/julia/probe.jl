top = 1
function g(a, b::Int, c=1, xs...; k=2, kw::Int=3) where T
    local loc
    x = a()
    y::Int = 2
    (p, q) = pair()
    for i in 1:3, j in 1:2
        i(); j()
    end
    for (u, v) in zs
        u()
    end
    [w() for w in fs]
    let lt = 1, lt2
        lt()
    end
    try
        go()
    catch e
        e()
    end
    map(fs) do h
        h()
    end
    f1 = z -> z()
    f2 = (m, n) -> m(n)
    global gl = 3
end
short(s, (d1, d2)) = s(d1)
