old : target/remediation/bins/WS1-before (cgg 0.10.2)
new : target/remediation/bins/WS1-after-v7 (cgg 0.10.2)
runs: median of 5, alternating; load: 2.93, 4.69, 7.06

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| groovy-gradle | 11530 | 11469 | -61 | -0.5 | noise(<1s) |
| app-druid-jaxrs | 8954 | 8330 | -624 | -7.0 | noise(<1s) |
| app-immich-nestjs | 791 | 797 | +6 | +0.8 | noise(<1s) |
| app-ghost-express | 1892 | 1896 | +4 | +0.2 | noise(<1s) |
| app-gin-photoprism | 1200 | 1157 | -43 | -3.6 | noise(<1s) |
| app-wordpress-android | 2928 | 2920 | -8 | -0.3 | noise(<1s) |
| cmake-kitware | 3432 | 3310 | -122 | -3.6 | noise(<1s) |
| cpp-nlohmann-json | 803 | 742 | -61 | -7.6 | noise(<1s) |
| app-rn-expensify | 6926 | 7206 | +280 | +4.0 | noise(<1s) |
| app-metabase-compojure | 6831 | 6969 | +138 | +2.0 | noise(<1s) |
| app-saleor-celery | 1964 | 1919 | -45 | -2.3 | noise(<1s) |
| app-orleans-samples | 1303 | 1412 | +109 | +8.4 | noise(<1s) |

total: 48554 ms -> 48127 ms (-427 ms, -0.9%)
elapsed: 583s
