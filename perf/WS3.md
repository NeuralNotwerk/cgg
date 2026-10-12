old : target/remediation/bins/WS3-before (cgg 0.10.2)
new : target/remediation/bins/WS3-after-v9 (cgg 0.10.2)
runs: median of 5, alternating; load: 3.40, 10.48, 15.18

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| app-wordpress-android | 2902 | 2950 | +48 | +1.7 | noise(<1s) |
| app-nextcloud-android | 609 | 619 | +10 | +1.6 | noise(<1s) |
| kotlin-okio | 285 | 300 | +15 | +5.3 | noise(<1s) |
| java-okhttp | 508 | 526 | +18 | +3.5 | noise(<1s) |
| groovy-gradle | 11599 | 11387 | -212 | -1.8 | noise(<1s) |
| app-druid-jaxrs | 8454 | 8357 | -97 | -1.1 | noise(<1s) |
| cmake-kitware | 3404 | 3352 | -52 | -1.5 | noise(<1s) |
| app-orleans-samples | 1304 | 1322 | +18 | +1.4 | noise(<1s) |
| csharp-newtonsoft | 576 | 587 | +11 | +1.9 | noise(<1s) |
| app-basedosdados-prefect | 873 | 874 | +1 | +0.1 | noise(<1s) |
| app-django-netbox | 3281 | 3294 | +13 | +0.4 | noise(<1s) |
| app-lemmy-actix | 307 | 310 | +3 | +1.0 | noise(<1s) |

total: 34102 ms -> 33878 ms (-224 ms, -0.7%)
elapsed: 411s
