module A
let outer () =
    let helper () = ()
    helper ()

let other () =
    helper ()
