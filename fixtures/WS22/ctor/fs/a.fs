module M
type Base(n: int) =
  member this.N = n
type Box(x: int) =
  inherit Base(x)
  new() = Box(1)
  member this.X = x
let make () = Box(2)
let make2 () = new Box()
