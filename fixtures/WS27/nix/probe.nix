{ pkgs, lib ? pkgs.lib, ... }@args:
let
  f = x: x;
  inherit (lib) mkIf;
  inherit g;
  h = { a, b ? a, ... }: a b;
in
rec {
  r = f 1;
  s = y: r y;
  t = with lib; mkOption {};
  u = args@{ c, ... }: c args;
}
