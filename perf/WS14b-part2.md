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
