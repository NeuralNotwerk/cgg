module A.B.Top

let f x = x

module Inner =
    let g y = y
    type T() =
        member _.M() = 1
