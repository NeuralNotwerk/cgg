let rec ra x = rb x
and rb (y: int) : int = ra y
and [<Foo>] rc z = z
let getCol (s: int) : int = s
let private pf (x: int) : int = x
let inline inf (x: int) : int = x
let useAll () = getCol 1 + pf 2 + inf 3 + rc 4
let mutable mv = 1
