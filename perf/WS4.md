old : target/remediation/bins/WS4-before (cgg 0.10.2)
new : target/remediation/bins/WS4-after-v11 (cgg 0.10.2)
runs: median of 5, alternating; load: 3.09, 6.68, 8.96

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| app-druid-jaxrs | 8110 | 8186 | +76 | +0.9 | noise(<1s) |
| groovy-gradle | 11232 | 11291 | +59 | +0.5 | noise(<1s) |
| zig-zig | 34336 | 34991 | +655 | +1.9 | noise(<1s) |
| app-tigerbeetle-zig | 417 | 436 | +19 | +4.6 | noise(<1s) |
| app-orleans-samples | 1241 | 1238 | -3 | -0.2 | noise(<1s) |
| csharp-newtonsoft | 564 | 572 | +8 | +1.4 | noise(<1s) |
| app-ghost-express | 1857 | 1955 | +98 | +5.3 | noise(<1s) |
| app-calcom-nextjs | 4529 | 4618 | +89 | +2.0 | noise(<1s) |
| app-django-netbox | 3163 | 3223 | +60 | +1.9 | noise(<1s) |
| app-gin-photoprism | 1082 | 1086 | +4 | +0.4 | noise(<1s) |

total: 66531 ms -> 67596 ms (+1065 ms, +1.6%)
elapsed: 807s
