# Found while working: QR9

- 25 November 2026, QR9: The TB89/D73 reservation conflict is resolved. Q84 keeps TB89/D73, and QR26 uses TB90/D74. QR9 adds no TB or D number.
- 25 November 2026, QR9: A replacement CreateAgent removes the previous session and its seed before registering the new seed. If registration fails (for example, a seed path already belongs to another agent), the old circuit has already been disabled. A follow-up should test the occupied-path case and preserve the prior session if the new registration fails.
- 25 November 2026, QR9 merge with #267: the first local workspace check failed in `failed_http_exchange_makes_the_cli_exit_unsuccessfully` at `crates/ferroan-capture/tests/http_proxy.rs:99`. Reading the proxy's deliberately failed HTTP exchange returned Linux `ConnectionReset` before the test could check the CLI's exit code. The full local rerun passed all 1,015 tests. Check whether the test should accept an empty response ending in a reset as well as EOF.
