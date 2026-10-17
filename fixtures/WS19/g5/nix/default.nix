let lib = import ./lib/d.nix; in { main = x: lib.dupn x; }
