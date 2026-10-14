old : target/remediation/bins/WS14b-before (cgg 0.10.2)
new : target/remediation/bins/WS14b-after-v9 (cgg 0.10.2)
runs: median of 5, alternating; load: 9.55, 12.99, 15.60

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| zig-zig | 35903 | 36386 | +483 | +1.3 | noise(<1s) |
| scala-spark | 9028 | 9695 | +667 | +7.4 | noise(<1s) |
| app-ghostty-zig | 5156 | 5221 | +65 | +1.3 | noise(<1s) |

total: 50087 ms -> 51302 ms (+1215 ms, +2.4%)
elapsed: 615s
