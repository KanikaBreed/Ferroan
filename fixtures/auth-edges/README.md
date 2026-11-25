# Authentication edge cases

`cargo xtask http auth-generate` writes [cases.json](cases.json). `cargo xtask http auth-verify` regenerates it in memory and compares the complete file. Both use the region server pin in [REFERENCES.md](../../docs/REFERENCES.md).

At Pin8, the authorization cases call `AuthorizationServerPostHandler.ProcessRequest` with a service that permits the request. The fixture preserves the response text, including the UTF-8 BOM. An XML deserialization exception is recorded as HTTP 500 with an empty body. The form case calls `AuthenticationServerPostHandler.ProcessRequest` and records the value actually stored for `AccountType[]=`.

The web-key case calls the pinned `WebkeyAuthenticationService.Authenticate` with an in-memory data plugin that returns false from the clear. The fixture records the number of attempted clears and whether a token was issued. All generated credential values stay in memory. Ferroan's SQLite test exercises the same refused write and checks for an empty authentication result and zero token rows (TB94, D79).
