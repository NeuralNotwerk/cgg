old : target/remediation/bins/WS4-before (cgg 0.10.2)
new : target/remediation/bins/WS4-after-v5 (cgg 0.10.2)
runs: median of 5, alternating; load: 5.35, 9.32, 11.15

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| app-ghost-express | 1951 | 2071 | +120 | +6.2 | noise(<1s) |
| app-rn-expensify | 6699 | 6794 | +95 | +1.4 | noise(<1s) |
| app-calcom-nextjs | 4597 | 4566 | -31 | -0.7 | noise(<1s) |
| app-metabase-compojure | 6418 | 6480 | +62 | +1.0 | noise(<1s) |
| app-django-netbox | 3254 | 3249 | -5 | -0.2 | noise(<1s) |
| app-hypothesis-pyramid | 2141 | 2135 | -6 | -0.3 | noise(<1s) |
| app-humhub-yii | 1202 | 1189 | -13 | -1.1 | noise(<1s) |
| app-ravada-mojolicious | 560 | 571 | +11 | +2.0 | noise(<1s) |
| perl-mojolicious | 435 | 424 | -11 | -2.5 | noise(<1s) |

total: 27257 ms -> 27479 ms (+222 ms, +0.8%)
elapsed: 330s
old : target/remediation/bins/WS4-before (cgg 0.10.2)
new : target/remediation/bins/WS4-after-v5 (cgg 0.10.2)
runs: median of 5, alternating; load: 7.20, 7.98, 10.03

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| app-gin-photoprism | 1129 | 1104 | -25 | -2.2 | noise(<1s) |
| app-ghcli-cobra | 518 | 519 | +1 | +0.2 | noise(<1s) |
| hcl-terraform-aws | 17559 | 17592 | +33 | +0.2 | noise(<1s) |
| app-druid-jaxrs | 8320 | 8376 | +56 | +0.7 | noise(<1s) |

total: 27526 ms -> 27591 ms (+65 ms, +0.2%)
elapsed: 332s
