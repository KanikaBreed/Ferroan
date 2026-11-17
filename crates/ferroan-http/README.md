# ferroan-http

Robust form bodies, `<ServerResponse>` documents and LLSD over HTTP.

`src/query.rs` and `src/xml.rs` follow Tranquillity `Source/OpenSim.Server.Base/ServerUtils.cs` at `713857f10b8e91a639a967d4354a4682b080793c`:

- `ParseQueryString` and `BuildQueryString`
- `BuildXmlResponse` and `ParseXmlResponse`

The comparisons are `fixtures/http`, written by `cargo xtask http` calling that C#. `ServerUtils.cs` is not copied. Percent-encoding is what `HttpUtility.UrlEncode` / `UrlDecode` do when those methods call them: UTF-8, space as `+`, lowercase hex, and the safe set `!()*-.0-9A-Z_a-z`. Nested maps in a query string are compact JSON, which is what LitJson writes.

## Limits

| Limit | Value | Tranquillity |
| --- | --- | --- |
| XML characters | 50_000_000 | `XmlReaderSettings.MaxCharactersInDocument` |
| XML depth | 64 | none; the reader recurses |
| Query bytes | 1 MiB | none |
| Fields in one map | 4096 | none |
| HTTP body | 50_000_000 bytes | the server's own buffering |

Over a limit, Ferroan returns `HttpError::Limit`. Tranquillity's XML parser returns an empty dictionary when the reader throws, including when the character cap is hit. A malformed document still becomes an empty map here, matching that swallow. Depth is the extra refusal: a 64-deep reply is not a document a connector sends.

`BuildQueryString` throws on a null value (`NullReferenceException`). `BuildXmlResponse` skips nulls and throws `ArgumentException` on an empty element name. Both are errors here.

## LLSD

Request bodies are classified the way `OSDParser.Deserialize(Stream)` classifies them, case-sensitive prefixes from libopenmetaverse `a3596616`: `<llsd>`, `<?xml`, `<? llsd/xml`, `<? llsd/notatio`, `<? llsd/binary`. The binary serializer writes `<?llsd/binary?>` with no space, which that stream check misses and then tries as JSON. Ferroan accepts both headers and decodes them with `ferroan-llsd`.

A body that matches none of those is JSON in Tranquillity. `ferroan-llsd` has no JSON codec, so the call returns `HttpError::Unsupported`. Response selection follows `BaseHttpServer.BuildLLSDResponse` for XML types and refuses `application/llsd+json` and `application/json` for the same reason. The fallback, and the usual Robust reply, is informal LLSD XML (`to_xml`).

## HTTP

`form_router` is an axum `POST` that reads a form body and writes `text/xml`. `post_form` and `post_llsd_xml` are the reqwest client. No viewer was pointed at this crate. The oracle fixtures are the acceptance check. CI does not run the C# oracle.
