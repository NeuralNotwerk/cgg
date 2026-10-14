old : target/remediation/bins/WS8-before (cgg 0.10.2)
new : target/remediation/bins/WS8-after-v8 (cgg 0.10.2)
runs: median of 5, alternating; load: 2.99, 15.69, 29.43

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| erlang-otp | 7976 | 7537 | -439 | -5.5 | noise(<1s) |
| dart-flutter | 9727 | 9195 | -532 | -5.5 | noise(<1s) |
| scala-spark | 9442 | 10231 | +789 | +8.4 | noise(<1s) |
| csharp-newtonsoft | 589 | 590 | +1 | +0.2 | noise(<1s) |
| lua-neovim | 901 | 1018 | +117 | +13.0 | noise(<1s) |
| php-laravel | 1929 | 1827 | -102 | -5.3 | noise(<1s) |
| app-rails-mastodon | 1677 | 1689 | +12 | +0.7 | noise(<1s) |
| c-redis | 600 | 650 | +50 | +8.3 | noise(<1s) |
| zig-zig | 35385 | 34857 | -528 | -1.5 | noise(<1s) |
| julia-genie | 133 | 132 | -1 | -0.8 | noise(<1s) |

total: 68359 ms -> 67726 ms (-633 ms, -0.9%)
elapsed: 828s
