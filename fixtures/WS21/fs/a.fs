module M
type Box(x: int) =
  new() = Box(1)
  member this.X = x
let make () = Box(2)
