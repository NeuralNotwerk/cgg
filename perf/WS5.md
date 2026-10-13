old : target/remediation/bins/WS5-before (cgg 0.10.2)
new : target/remediation/bins/WS5-after-v3 (cgg 0.10.2)
runs: median of 5, alternating; load: 9.57, 12.15, 15.77

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| groovy-gradle | 11786 | 12061 | +275 | +2.3 | noise(<1s) |
| app-druid-jaxrs | 8796 | 8854 | +58 | +0.7 | noise(<1s) |
| scala-spark | 8915 | 9155 | +240 | +2.7 | noise(<1s) |
| dart-flutter | 9797 | 9710 | -87 | -0.9 | noise(<1s) |
| app-calcom-nextjs | 4679 | 4669 | -10 | -0.2 | noise(<1s) |
| app-django-netbox | 3574 | 3474 | -100 | -2.8 | noise(<1s) |
| rust-bat | 1419 | 1362 | -57 | -4.0 | noise(<1s) |
| app-orleans-samples | 1293 | 1313 | +20 | +1.5 | noise(<1s) |
| csharp-newtonsoft | 572 | 579 | +7 | +1.2 | noise(<1s) |
| app-alarik-vapor | 240 | 253 | +13 | +5.4 | noise(<1s) |

total: 51071 ms -> 51430 ms (+359 ms, +0.7%)
elapsed: 617s

### --dead-code mode (implies --dynamic-dispatch), WS5-before vs WS5-after-v4, median of 5, alternating; load: 26.57, 21.17, 17.32

| repo | old ms | new ms | delta ms | delta % |
|---|---|---|---|---|
| groovy-gradle | 15797 | 16948 | 1151 | +7.3 |
| app-druid-jaxrs | 10409 | 11564 | 1155 | +11.1 |
| csharp-newtonsoft | 659 | 713 | 54 | +8.2 |
| app-django-netbox | 4165 | 4324 | 159 | +3.8 |
