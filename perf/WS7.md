old : target/remediation/bins/WS7-before (cgg 0.10.2)
new : target/remediation/bins/WS7-after-v3 (cgg 0.10.2)
runs: median of 5, alternating; load: 5.95, 10.49, 11.74

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| app-druid-jaxrs | 8532 | 8499 | -33 | -0.4 | noise(<1s) |
| groovy-gradle | 11768 | 11714 | -54 | -0.5 | noise(<1s) |
| app-thingsboard-concurrent | 5276 | 5249 | -27 | -0.5 | noise(<1s) |
| app-jabref-javafx | 1357 | 1368 | +11 | +0.8 | noise(<1s) |
| app-orleans-samples | 1273 | 1284 | +11 | +0.9 | noise(<1s) |
| csharp-newtonsoft | 588 | 602 | +14 | +2.4 | noise(<1s) |
| c-redis | 605 | 603 | -2 | -0.3 | noise(<1s) |
| cmake-kitware | 3475 | 3753 | +278 | +8.0 | noise(<1s) |
| app-cuda-samples | 729 | 712 | -17 | -2.3 | noise(<1s) |

total: 33603 ms -> 33784 ms (+181 ms, +0.5%)
elapsed: 409s

## --dead-code --include-tests (paired, 5 runs)
load: 7.73 8.99 10.68
| repo | before ms | after ms | delta ms | delta % |
|---|---|---|---|---|
| app-jabref-javafx | 1703 | 1600 | -103 | -6.0 |
| app-thingsboard-concurrent | 6995 | 7014 | +19 | +0.3 |
| app-orleans-samples | 1713 | 1741 | +28 | +1.6 |
