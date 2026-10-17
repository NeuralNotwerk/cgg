let outer () =
  let go () = () in
  go ()

let other () =
  let f = fun go -> go () in
  f (fun () -> ())
