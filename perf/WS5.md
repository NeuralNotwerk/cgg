old : target/remediation/bins/WS5-before (cgg 0.10.2)
new : target/remediation/bins/WS5-after-v3 (cgg 0.10.2)
runs: median of 5, alternating; load: 9.57, 12.15, 15.77

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| groovy-gradle | 11786 | 12061 | +275 | +2.3 | noise(<1s) |
| app-druid-jaxrs | 8796 | 8854 | +58 | +0.7 | noise(<1s) |
| scala-spark | 8915 | 9155 | +240 | +2.7 | noise(<1s) |
