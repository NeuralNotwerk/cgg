# WS14b perf (WS14b-before vs WS14b-after-v9, median of 5, alternating)

## Part 1
old : target/remediation/bins/WS14b-before (cgg 0.10.2)
new : target/remediation/bins/WS14b-after-v9 (cgg 0.10.2)
runs: median of 5, alternating; load: 9.55, 12.99, 15.60

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| zig-zig | 35903 | 36386 | +483 | +1.3 | noise(<1s) |
| scala-spark | 9028 | 9695 | +667 | +7.4 | noise(<1s) |
| app-ghostty-zig | 5156 | 5221 | +65 | +1.3 | noise(<1s) |

total: 50087 ms -> 51302 ms (+1215 ms, +2.4%)
elapsed: 615s

## Part 2
old : target/remediation/bins/WS14b-before (cgg 0.10.2)
new : target/remediation/bins/WS14b-after-v9 (cgg 0.10.2)
runs: median of 5, alternating; load: 5.63, 10.85, 9.69

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| app-wordpress | 2210 | 2502 | +292 | +13.2 | noise(<1s) |
| groovy-gradle | 11765 | 11610 | -155 | -1.3 | noise(<1s) |
| app-ravada-mojolicious | 479 | 494 | +15 | +3.1 | noise(<1s) |
| perl-mojolicious | 386 | 389 | +3 | +0.8 | noise(<1s) |
| ocaml-dune | 1278 | 1281 | +3 | +0.2 | noise(<1s) |
| app-vaultwarden-rocket | 245 | 254 | +9 | +3.7 | noise(<1s) |

total: 16363 ms -> 16530 ms (+167 ms, +1.0%)
elapsed: 213s

Combined: 9 repositories, 66,450 ms -> 67,832 ms (+1,382 ms, +2.1%); no repository slower by 1 s or more. Perl and Rust repos in the corpus all run under 1 s (ravada 0.5 s is the largest Perl).
