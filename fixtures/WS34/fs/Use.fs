module App.Use
open A.B
open App.CodeFixes

let run () =
    let a = Top.f 1
    let b = Top.Inner.g 2
    let c = N.S.Mod.h 3
    let d = AddA.fix 4 + AddB.fix 5
    let e = List.map id [1]
    a + b + c + d
