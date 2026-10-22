module Forms

let plain x = x
let typed (a: int) b = a + b
let tupled (a, b) = a + b
let inline inl x = x
let private priv (x: int) = x
let rec ra x = rb x
and rb (y: int) = ra y
let value = 1
let lam = fun x -> x
let outer () =
    let inner (z: int) = z
    inner 1

type T(n: int) =
    let classLet (q: int) = q
    member this.M(x) = x
    member _.N (x: int) y = x
    static member S (x: int) = x
    override this.ToString() = "t"
    abstract member A : int -> int
    default this.A x = x
    member val Auto = 1 with get, set
    member this.Prop = 1
