# Configuration fixtures

Resolved key/value sets from `cargo xtask config generate`. The generator runs `tools/config-oracle`, which calls Nini 0.9.2.16 and Tranquillity's include walk at the revision in `manifest.txt`. Do not edit the files by hand.

`parse-crlf/source.ini` is stored so a checkout keeps CRLF. The other files use LF.

`manifest.txt` lists the case ids. Each case has `kind.txt` (`parse`, `expand`, `region`, `grid`, `region-ref` or `grid-ref`). A success writes `resolved.txt` and `warnings.txt`. A failure writes `error.txt` as `code` or `code` plus a tab and a detail. Values escape `\`, newlines and tabs. Rows are sorted by the escaped line, ordinal.

Cases of kind `region-ref` and `grid-ref` read the shipped INI files from the pinned checkout (`FERROAN_REF`, default `~/Git/reference`). Rust skips those two when the checkout is absent, including in ordinary CI. The Linux live workflow has the checkout and verifies every case through the C# oracle. The other cases use the small files committed next to them.

The shipped dumps contain placeholder passwords and hostnames from Tranquillity's public examples (`ConsolePass` = `secret`, certificate and voice passwords, `BaseHostname` = `127.0.0.1`). They are the published examples, not live credentials. The generator refuses a resolved value that contains this machine's home directory.

`region-shipped` warns that `config-include/osslEnable.ini` and `config-include/StandaloneCommon.ini` are missing. Those files ship as `.example` only. `grid-shipped` has no missing include.
