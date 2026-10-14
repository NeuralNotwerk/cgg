old : target/remediation/bins/WS9-before (cgg 0.10.2)
new : target/remediation/bins/WS9-after-v2 (cgg 0.10.2)
runs: median of 5, alternating; load: 3.75, 5.51, 7.91

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| hcl-terraform-aws | 17625 | 6566 | -11059 | -62.7 |  |
| zig-zig | 35081 | 35712 | +631 | +1.8 | noise(<1s) |
| cpp-nlohmann-json | 690 | 719 | +29 | +4.2 | noise(<1s) |
| cpp-spdlog | 184 | 174 | -10 | -5.4 | noise(<1s) |
| csharp-newtonsoft | 609 | 581 | -28 | -4.6 | noise(<1s) |
| dart-flutter | 9112 | 9180 | +68 | +0.7 | noise(<1s) |
| scala-spark | 10546 | 9959 | -587 | -5.6 | noise(<1s) |
| go-caddy | 206 | 205 | -1 | -0.5 | noise(<1s) |
| swift-alamofire | 187 | 186 | -1 | -0.5 | noise(<1s) |
| php-laravel | 1818 | 1614 | -204 | -11.2 | noise(<1s) |
| haskell-pandoc | 597 | 592 | -5 | -0.8 | noise(<1s) |
| julia-flux | 63 | 64 | +1 | +1.6 | noise(<1s) |
| erlang-otp | 7470 | 7493 | +23 | +0.3 | noise(<1s) |
| fsharp-paket | 353 | 354 | +1 | +0.3 | noise(<1s) |

total: 84541 ms -> 73399 ms (-11142 ms, -13.2%)
elapsed: 970s
