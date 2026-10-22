{ pkgs, cb ? null, ... }@args:
let
  # A `let` function: calls bind it.
  helper = x: x + 1;
in
rec {
  # Functions whose names the bindings below reuse.
  step = x: x;
  handler = x: x;
  item = x: x;
  fmt = x: x;

  # A `let` function and a `rec` attribute keep their edges.
  useHelper = y: helper y;
  useFmt = y: fmt y;
  # A lambda parameter, a formal set, a `let` inside, the file's formal.
  withParam = step: step 2;
  withFormals = { handler, ... }: handler 3;
  withLet = y: let item = y; in item 4;
  withFileFormal = y: cb y;
  # `with` never shadows a lexical name, nor names one.
  dynamic = y: with pkgs; mkThing y;
  # A qualified select is never shadowed by a local of its last name.
  selected = step: pkgs.step step;
  # An `inherit` from an import names that file's function.
  withImport = y: let inherit (import ./lib.nix { }) mkPath; in mkPath y;
  # Two functions' own `let` functions of one name: each binds its own.
  twinA = y: let pick = v: v; in pick y;
  twinB = y: let pick = v: v + 1; in pick y;
}
