old : target/remediation/bins/WS8-before (cgg 0.10.2)
new : target/remediation/bins/WS8-after-v7 (cgg 0.10.2)
runs: median of 5, alternating; load: 3.04, 6.11, 8.14

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| erlang-otp | 7852 | 7940 | +88 | +1.1 | noise(<1s) |
| dart-flutter | 9671 | 10357 | +686 | +7.1 | noise(<1s) |
| scala-spark | 9479 | 13350 | +3871 | +40.8 |  |
| csharp-newtonsoft | 595 | 601 | +6 | +1.0 | noise(<1s) |
| lua-neovim | 876 | 1036 | +160 | +18.3 | noise(<1s) |
| php-laravel | 1914 | 1916 | +2 | +0.1 | noise(<1s) |
| app-rails-mastodon | 1659 | 1799 | +140 | +8.4 | noise(<1s) |
| c-redis | 609 | 657 | +48 | +7.9 | noise(<1s) |
| zig-zig | 35937 | 36991 | +1054 | +2.9 |  |
| julia-genie | 134 | 133 | -1 | -0.7 | noise(<1s) |

total: 68726 ms -> 74780 ms (+6054 ms, +8.8%)
elapsed: 863s
