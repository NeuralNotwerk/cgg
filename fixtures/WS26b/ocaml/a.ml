let handler x = x + 1
let unused_by_value y = y
let run xs = List.map handler xs
let shadow handler = List.map handler [1]
let table = [ handler; Other.helper ]
let piped xs = xs |> List.iter Other.helper
let c () = Other.helper 1
