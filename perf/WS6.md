old : target/remediation/bins/WS6-before (cgg 0.10.2)
new : target/remediation/bins/WS6-after-v3 (cgg 0.10.2)
runs: median of 5, alternating; load: 3.20, 8.00, 9.78

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| groovy-gradle | 11531 | 11769 | +238 | +2.1 | noise(<1s) |
| app-druid-jaxrs | 8398 | 8938 | +540 | +6.4 | noise(<1s) |
| app-jabref-javafx | 1329 | 1378 | +49 | +3.7 | noise(<1s) |
| app-orleans-samples | 1280 | 1269 | -11 | -0.9 | noise(<1s) |
| csharp-newtonsoft | 588 | 578 | -10 | -1.7 | noise(<1s) |
| app-django-netbox | 3557 | 3470 | -87 | -2.4 | noise(<1s) |
| py-scrapy | 290 | 295 | +5 | +1.7 | noise(<1s) |
| py-litestar | 365 | 366 | +1 | +0.3 | noise(<1s) |

total: 27338 ms -> 28063 ms (+725 ms, +2.7%)
elapsed: 334s

### --dead-code mode, WS6-before vs WS6-after-v3, median of 5, alternating; load: 5.80, 7.90, 9.28

| repo | old ms | new ms | delta ms | delta % |
|---|---|---|---|---|
| groovy-gradle | 15236 | 15377 | 141 | +0.9 |
| app-druid-jaxrs | 9881 | 10258 | 377 | +3.8 |
| app-jabref-javafx | 1499 | 1541 | 42 | +2.8 |
| app-django-netbox | 4401 | 4251 | -150 | -3.4 |
| app-orleans-samples | 1627 | 1691 | 64 | +3.9 |

No deviation >= 1 s in either mode.
