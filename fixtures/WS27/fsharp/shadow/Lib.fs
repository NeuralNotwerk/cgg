module Lib

// Functions whose names the bindings below reuse.
let cb x = x
let handler x = x
let item x = x
let step x = x
let err x = x
let now () = 0

// A parameter, a lambda, a match rule, a local, a `for` target, `try`.
let withParam cb = cb 1
let withLambda xs = List.map (fun handler -> handler 2) xs
let withMatch o =
    match o with
    | Some item -> item 3
    | None -> 0
let withLocal () =
    let step = now
    step ()
let withTry () =
    try now () with err -> err ()

// A nested function keeps its edge; positional shadowing still picks it.
let withNested x =
    let helper y = y + 1
    helper x
let helper y = y
let useHelper x = helper x

// A qualified call is never shadowed; a value sees the module function.
let withQualified cb = Lib.cb cb
let withValue () =
    let now = now ()
    now
