struct Foo{T}
    x::T
end
f(::Type{T}, x) where {T} = x
function g(x)
    Foo{Int}(x)
    f(Int, x)
end
function p(x::T) where T
    h(x)
end
h(x)::Int = x
