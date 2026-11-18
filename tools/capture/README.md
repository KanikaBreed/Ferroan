# Tranquillity capture patch

`tranquillity-capture.patch` records raw UDP datagrams from a local Tranquillity build. The steps, the file format and the redaction rules are in [docs/CAPTURES.md](../../docs/CAPTURES.md).

The patch is applied with `git apply` to a scratch clone of OpenSim-Tranquillity at the revision in [docs/REFERENCES.md](../../docs/REFERENCES.md). It is not applied to the reference checkout and it is not sent upstream.

`DatagramCapture.cs` in the patch is new Ferroan code. The edits to `OpenSimUDPBase.cs` and `LLUDPServer.cs` sit on top of those files' existing BSD notices.
