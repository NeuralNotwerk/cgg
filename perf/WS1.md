old : target/remediation/bins/WS1-before (cgg 0.10.2)
new : target/remediation/bins/WS1-after-v4 (cgg 0.10.2)
runs: median of 5, alternating; load: 3.99, 12.49, 20.08

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| groovy-gradle | 11607 | 11402 | -205 | -1.8 | noise(<1s) |
| app-druid-jaxrs | 9125 | 8412 | -713 | -7.8 | noise(<1s) |
| app-immich-nestjs | 794 | 800 | +6 | +0.8 | noise(<1s) |
| app-ghost-express | 1887 | 1964 | +77 | +4.1 | noise(<1s) |
| app-gin-photoprism | 1180 | 1157 | -23 | -1.9 | noise(<1s) |
| app-wordpress-android | 3003 | 2953 | -50 | -1.7 | noise(<1s) |
| cmake-kitware | 3383 | 3388 | +5 | +0.1 | noise(<1s) |
| app-rn-expensify | 6658 | 6736 | +78 | +1.2 | noise(<1s) |
| app-metabase-compojure | 6281 | 6594 | +313 | +5.0 | noise(<1s) |
| app-saleor-celery | 2016 | 1947 | -69 | -3.4 | noise(<1s) |
| app-orleans-samples | 1290 | 1323 | +33 | +2.6 | noise(<1s) |

total: 47224 ms -> 46676 ms (-548 ms, -1.2%)
elapsed: 570s
