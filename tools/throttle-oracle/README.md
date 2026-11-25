# Throttle oracle

`cargo xtask throttle generate` writes `fixtures/throttle/rates.json`.
`cargo xtask throttle verify` rebuilds those rates and requires the same bytes.

The tool compiles `TokenBucket.cs` and `ThrottleRates.cs` from the pinned
Tranquillity checkout and reads the bucket properties. It does not copy those
sources into Ferroan and it does not start a region. The send-clock cases stay
in the Rust tests, because the C# drip reads the wall clock.
