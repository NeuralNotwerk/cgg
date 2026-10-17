let outer () =
  let helper () = () in
  helper ()

let other () =
  helper ()
