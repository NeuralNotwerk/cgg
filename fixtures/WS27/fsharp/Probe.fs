module Probe

let top = 1

let f (a: int) b (c, d) ?opt =
    let x = a
    let rec loop n = loop n
    use r = open' ()
    for i in xs do i ()
    for j = 1 to 3 do j ()
    let g = fun p q -> p q
    match b with
    | Some v when v > 0 -> v ()
    | _ -> ()
    try go () with e -> e ()

let h () = async {
    let! res = get ()
    return res ()
}

type T() =
    let priv x = x
    member this.M(y) = priv y
