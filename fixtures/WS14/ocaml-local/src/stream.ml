let iter f s =
  let rec go n = if n > 0 then (f n; go (n - 1)) in
  go s

let map f xs =
  let k x = f x in
  List.map k xs
