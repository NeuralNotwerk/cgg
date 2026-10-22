module A
let handler x = x + 1
let run xs = List.map handler xs
let shadow handler = List.map handler [1]
let piped xs = xs |> List.map B.helper |> List.sum
let step a b = a + b
let pipeCall xs = xs |> step 1
let c () = B.helper 1
