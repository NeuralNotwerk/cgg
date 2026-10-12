old : target/remediation/bins/WS2-before (cgg 0.10.2)
new : target/remediation/bins/WS2-after-v1 (cgg 0.10.2)
runs: median of 5, alternating; load: 8.45, 12.81, 13.78

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| groovy-gradle | 11474 | 11796 | +322 | +2.8 | noise(<1s) |
| app-druid-jaxrs | 8967 | 8732 | -235 | -2.6 | noise(<1s) |
| java-spring-batch | 651 | 686 | +35 | +5.4 | noise(<1s) |
| app-spring-mall | 422 | 406 | -16 | -3.8 | noise(<1s) |
| app-orleans-samples | 1359 | 1344 | -15 | -1.1 | noise(<1s) |
| app-ombi-quartz | 639 | 648 | +9 | +1.4 | noise(<1s) |
| csharp-newtonsoft | 595 | 614 | +19 | +3.2 | noise(<1s) |
| app-eshop-aspnet | 171 | 176 | +5 | +2.9 | noise(<1s) |
| app-metabase-compojure | 6550 | 6719 | +169 | +2.6 | noise(<1s) |
| app-rn-expensify | 6961 | 6725 | -236 | -3.4 | noise(<1s) |
| app-calcom-nextjs | 4929 | 4779 | -150 | -3.0 | noise(<1s) |

total: 42718 ms -> 42625 ms (-93 ms, -0.2%)
elapsed: 518s
