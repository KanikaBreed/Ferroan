# Throttle oracle

`cargo xtask throttle generate` writes `fixtures/throttle/rates.json`.
`cargo xtask throttle verify` rebuilds those rates and requires the same bytes.

The tool references the pinned `OpenSim.Region.ClientStack.LindenUDP` project.
It builds each client with the real `LLUDPClient` constructor under a
`TokenBucket` scene parent, applies each blob with `SetThrottles`, and reads
`FlowThrottle` and the category buckets (`m_throttleCategories`, by
reflection). It does not copy those sources into Ferroan and it does not start
a region. The short-blob cases record what C# usually reads past the array
(TB84); Ferroan ignores those blobs and a malformed one (D68). The send-clock
cases stay in the Rust tests, because the C# drip reads the wall clock.
