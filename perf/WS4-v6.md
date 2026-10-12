old : target/remediation/bins/WS4-before (cgg 0.10.2)
new : target/remediation/bins/WS4-after-v7 (cgg 0.10.2)
runs: median of 5, alternating; load: 2.17, 5.12, 9.57

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| app-ghost-express | 1971 | 2045 | +74 | +3.8 | noise(<1s) |
| app-rn-expensify | 6783 | 7076 | +293 | +4.3 | noise(<1s) |
| app-calcom-nextjs | 4654 | 4591 | -63 | -1.4 | noise(<1s) |
| app-metabase-compojure | 6580 | 6520 | -60 | -0.9 | noise(<1s) |
| app-django-netbox | 3309 | 3313 | +4 | +0.1 | noise(<1s) |
| app-hypothesis-pyramid | 2136 | 2120 | -16 | -0.7 | noise(<1s) |
| app-humhub-yii | 1211 | 1204 | -7 | -0.6 | noise(<1s) |
| app-ravada-mojolicious | 548 | 560 | +12 | +2.2 | noise(<1s) |
| perl-mojolicious | 431 | 455 | +24 | +5.6 | noise(<1s) |

total: 27623 ms -> 27884 ms (+261 ms, +0.9%)
elapsed: 334s
old : target/remediation/bins/WS4-before (cgg 0.10.2)
new : target/remediation/bins/WS4-after-v7 (cgg 0.10.2)
runs: median of 5, alternating; load: 7.68, 7.47, 9.32

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| app-gin-photoprism | 1144 | 1135 | -9 | -0.8 | noise(<1s) |
| app-ghcli-cobra | 517 | 526 | +9 | +1.7 | noise(<1s) |
| hcl-terraform-aws | 17615 | 17522 | -93 | -0.5 | noise(<1s) |
| app-druid-jaxrs | 8284 | 8281 | -3 | -0.0 | noise(<1s) |

total: 27560 ms -> 27464 ms (-96 ms, -0.3%)
elapsed: 331s
