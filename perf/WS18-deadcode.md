old : target/remediation/bins/WS18-before (cgg 0.10.2)
new : target/remediation/bins/WS18-after-v5 (cgg 0.10.2)
runs: median of 5, alternating; load: 4.54, 5.22, 5.17

| repo | old ms | new ms | delta ms | delta % | note |
|---|---|---|---|---|---|
| neurosymbolic_kb | 908 | 977 | +69 | +7.6 | noise(<1s) |
| rust-ntex | 426 | 433 | +7 | +1.6 | noise(<1s) |
| app-orleans-samples | 1832 | 1838 | +6 | +0.3 | noise(<1s) |
| app-django-netbox | 4436 | 4443 | +7 | +0.2 | noise(<1s) |
| app-druid-jaxrs | 7172 | 7361 | +189 | +2.6 | noise(<1s) |
| erlang-otp | 9406 | 9426 | +20 | +0.2 | noise(<1s) |
| hcl-terraform-aws | 8205 | 8311 | +106 | +1.3 | noise(<1s) |
| app-symfony-messenger | 4481 | 4533 | +52 | +1.2 | noise(<1s) |
| app-kapua-camel | 1892 | 1915 | +23 | +1.2 | noise(<1s) |

total: 38758 ms -> 39237 ms (+479 ms, +1.2%)
elapsed: 471s
