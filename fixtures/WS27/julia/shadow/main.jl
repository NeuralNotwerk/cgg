# Functions whose names the parameters and locals below reuse.
cb(x) = x
handler(x) = x
helper(x) = x
item(x) = x
step(x) = x
err(x) = x
num(x) = x
later(x) = x
now() = 0

# Parameters: positional, keyword, short form, lambda, do block.
function with_param(cb; handler = identity)
    cb(1)
    handler(2)
end
with_short(item) = item(3)
with_lambda(xs) = map(step -> step(4), xs)
function with_do(xs)
    map(xs) do err
        err(5)
    end
end

# Locals: a function, a local function, a literal, a `for` variable.
function with_local()
    helper = x -> x * 2
    helper(6)
end
function with_literal()
    num = 1
    num(7)
end
function with_for(fs)
    for later in fs
        later(8)
    end
    later(9)
end

# A `global` assignment is not a local.
function with_global()
    global step = 1
    step(10)
end

# Positive control: the value sees the function.
function controls()
    now = now()
    now(11)
end

# A parameter's own default still sees the function.
function with_default(handler = handler(12))
    handler
end
