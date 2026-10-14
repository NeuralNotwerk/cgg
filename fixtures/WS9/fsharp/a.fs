module M
let f<'T> (x: 'T) = x
let g () =
    f<int> 1
    f 2
